//! Favorites reconciler (issue #347, step 9): push-before-pull sync between
//! the DB's per-server favorites and the remote, dispatched through the
//! [`MediaServerClient`](crate::client::MediaServerClient) trait so it's
//! service-agnostic.
//!
//! Push first so a just-toggled like isn't reverted by the pull; pending rows
//! that fail to push stay pending and are retried next cycle. The pull replaces
//! the clean set (dirty rows survive — see `replace_favorites_clean`).

use crate::client::{AuthOutcome, client_for};
use crate::server_ops::ServerConn;

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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncError {
    /// Real auth rejection — the caller should mark the server expired and
    /// surface re-auth UI. Distinct from a network blip.
    Expired,
    /// Transient (network/server) failure — back off and retry later.
    Unreachable(String),
}

/// One reconcile cycle for `server_id` against the connection's remote.
#[tracing::instrument(name = "favorites.reconcile", skip(db, conn), fields(server = %server_id, ?reason))]
pub async fn reconcile_favorites(
    db: &db::Db,
    conn: &ServerConn,
    server_id: &str,
    reason: SyncReason,
) -> Result<SyncReport, SyncError> {
    // Decide what there is to do BEFORE any network call — a reconcile with no
    // pending pushes and a fresh pull must be a complete no-op (not even the
    // validate request). The DB is the only thing consulted on the quiet path.
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
        matches!(reason, SyncReason::Manual) || now.saturating_sub(last_pull) >= PULL_MIN_SECS;
    if likes.is_empty() && unlikes.is_empty() && !should_pull {
        return Ok(SyncReport::default());
    }

    let client = client_for(conn);

    match client.validate().await {
        AuthOutcome::Valid => {}
        AuthOutcome::Expired => return Err(SyncError::Expired),
        AuthOutcome::Unreachable => {
            return Err(SyncError::Unreachable("server unreachable".into()));
        }
    }

    let mut report = SyncReport::default();

    // Push pending likes, then pending unlikes (each resolved on success only,
    // so a failure is retried next cycle).
    for r in likes {
        match client.set_favorite(&r, true).await {
            Ok(()) => {
                let _ = db.clear_favorite_dirty(server_id, &r).await;
                report.pushed_likes += 1;
            }
            Err(e) => {
                tracing::warn!(error = %e, item = %r, "favorite like push failed");
                report.failed_pushes += 1;
            }
        }
    }
    for r in unlikes {
        match client.set_favorite(&r, false).await {
            Ok(()) => {
                let _ = db.clear_favorite_dirty(server_id, &r).await;
                report.pushed_unlikes += 1;
            }
            Err(e) => {
                tracing::warn!(error = %e, item = %r, "favorite unlike push failed");
                report.failed_pushes += 1;
            }
        }
    }

    // Pull: the remote set becomes the clean baseline; still-pending local rows
    // survive. fetch_favorites is EXPENSIVE for YT (a full liked-library browse
    // stream), so the pull is staleness-gated (computed up top): Manual always
    // pulls; everything else only when the last pull is old.
    if should_pull {
        let remote = client
            .fetch_favorites()
            .await
            .map_err(SyncError::Unreachable)?;
        report.pulled = remote.len();
        db.replace_favorites_clean(server_id, &remote)
            .await
            .map_err(|e| SyncError::Unreachable(e.to_string()))?;
        let _ = db.meta_put("fav_pull", server_id, &now.to_string()).await;
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
