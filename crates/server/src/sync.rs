//! Push-before-pull favorites reconciliation through [`MediaSource`].
//!
//! Pending writes are pushed first so a pull cannot revert a recent toggle.
//! Failed writes remain pending; the pull replaces only clean favorites.

use crate::source::{AuthOutcome, MediaSource, SourceError};

/// Minimum age of the last remote pull before a non-Manual reconcile pulls
/// again. Pushes are never gated — only the expensive full fetch is.
const PULL_MIN_SECS: u64 = 30 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncReason {
    Activate,
    Interval,
    AfterMutation,
    Manual,
}

#[derive(Debug, Default, Clone)]
pub struct SyncReport {
    pub pushed_likes: usize,
    pub pushed_unlikes: usize,
    pub failed_pushes: usize,
    pub pulled: usize,
    /// Whether a pull APPLIED this cycle — `pulled` is the remote set's size,
    /// so a pull that only removed rows reports `pulled == 0` and the caller
    /// still needs to refresh the UI.
    pub did_pull: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncError {
    /// Real auth rejection — the caller should mark the server expired and
    /// surface re-auth UI. Distinct from a network blip.
    Expired,
    /// Transient (network/server) failure — back off and retry later.
    Unreachable(String),
}

/// One reconcile cycle for the source's server against its remote.
#[tracing::instrument(name = "favorites.reconcile", skip(source), fields(server = %source.source().as_str(), ?reason))]
pub async fn reconcile_favorites(
    source: &dyn MediaSource,
    reason: SyncReason,
) -> Result<SyncReport, SyncError> {
    let db = source.db();
    let server_id = source.source().as_str();

    let likes = db
        .dirty_favorites(server_id)
        .await
        .map_err(|e| SyncError::Unreachable(e.to_string()))?;
    let unlikes = db
        .dirty_unlikes(server_id)
        .await
        .map_err(|e| SyncError::Unreachable(e.to_string()))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let last_pull: u64 = db
        .meta_get("fav_pull", server_id)
        .await
        .ok()
        .flatten()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let should_pull =
        matches!(reason, SyncReason::Manual) || last_pull > now || now - last_pull >= PULL_MIN_SECS;
    if likes.is_empty() && unlikes.is_empty() && !should_pull {
        return Ok(SyncReport::default());
    }

    match source.validate().await {
        AuthOutcome::Valid => {}
        AuthOutcome::Expired => return Err(SyncError::Expired),
        AuthOutcome::Unreachable => {
            return Err(SyncError::Unreachable("server unreachable".into()));
        }
    }

    let mut report = SyncReport::default();

    let mut pushed_like_refs: Vec<String> = Vec::new();
    let mut pushed_unlike_refs: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    for r in likes {
        match source.push_favorite(&r, true).await {
            Ok(()) => {
                let _ = db.clear_favorite_dirty(server_id, &r).await;
                report.pushed_likes += 1;
                pushed_like_refs.push(r);
            }
            Err(e) => {
                tracing::warn!(error = %e, item = %r, "favorite like push failed");
                report.failed_pushes += 1;
            }
        }
    }
    for r in unlikes {
        match source.push_favorite(&r, false).await {
            Ok(()) => {
                let _ = db.clear_favorite_dirty(server_id, &r).await;
                report.pushed_unlikes += 1;
                pushed_unlike_refs.insert(r);
            }
            Err(e) => {
                tracing::warn!(error = %e, item = %r, "favorite unlike push failed");
                report.failed_pushes += 1;
            }
        }
    }

    if should_pull {
        let mut remote = source.fetch_favorites().await.map_err(|e| match e {
            SourceError::Auth => SyncError::Expired,
            other => SyncError::Unreachable(other.to_string()),
        })?;
        report.pulled = remote.len();

        for r in pushed_like_refs {
            if !remote.contains(&r) {
                remote.push(r);
            }
        }
        remote.retain(|r| !pushed_unlike_refs.contains(r));
        db.replace_favorites_clean(server_id, &remote)
            .await
            .map_err(|e| SyncError::Unreachable(e.to_string()))?;
        let _ = db.meta_put("fav_pull", server_id, &now.to_string()).await;
        report.did_pull = true;
    }

    tracing::info!(
        pushed_likes = report.pushed_likes,
        pushed_unlikes = report.pushed_unlikes,
        failed = report.failed_pushes,
        pulled = report.pulled,
        "favorites reconciled"
    );
    Ok(report)
}
