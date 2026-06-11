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
    let likes = db
        .dirty_favorites(server_id)
        .await
        .map_err(|e| SyncError::Unreachable(e.to_string()))?;
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
    let unlikes = db
        .dirty_unlikes(server_id)
        .await
        .map_err(|e| SyncError::Unreachable(e.to_string()))?;
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
    // survive.
    let remote = client
        .fetch_favorites()
        .await
        .map_err(SyncError::Unreachable)?;
    report.pulled = remote.len();
    db.replace_favorites_clean(server_id, &remote)
        .await
        .map_err(|e| SyncError::Unreachable(e.to_string()))?;

    tracing::info!(
        pushed_likes = report.pushed_likes,
        pushed_unlikes = report.pushed_unlikes,
        failed = report.failed_pushes,
        pulled = report.pulled,
        "favorites reconciled"
    );
    Ok(report)
}
