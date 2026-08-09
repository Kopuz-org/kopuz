//! Server-side play-queue sync (Subsonic `savePlayQueue`/`getPlayQueue`) —
//! the remote counterpart to `queue_state`'s local persistence, gated on
//! `Capabilities::play_queue` so it's a no-op for every other backend.

use crate::use_player_controller::PlayerController;
use config::MusicService;
use dioxus::prelude::*;
use reader::Track;

/// The `c=` client name kopuz sends on every Subsonic call (see
/// `server::subsonic`'s private `CLIENT_NAME`) — used to tell our own writes
/// apart from another client's when deciding whether to adopt the server queue.
const OWN_CLIENT_NAME: &str = "kopuz";

/// The queue's item ids, if every track is this source's own (a Subsonic or
/// Custom server track) — a mixed or local queue has no single remote queue
/// to save to.
fn server_item_ids(queue: &[Track]) -> Option<Vec<String>> {
    if queue.is_empty() {
        return None;
    }
    queue
        .iter()
        .map(|t| match t.id.service() {
            Some(MusicService::Subsonic) | Some(MusicService::Custom) => {
                Some(t.id.key().into_owned())
            }
            _ => None,
        })
        .collect()
}

/// The `(item_ids, current_id, position_ms)` to save, or `None` if the queue
/// isn't saveable (see [`server_item_ids`]).
pub fn save_payload(
    queue: &[Track],
    current_queue_index: usize,
    progress_secs: u64,
) -> Option<(Vec<String>, Option<String>, u64)> {
    let item_ids = server_item_ids(queue)?;
    let current_id = item_ids.get(current_queue_index).cloned();
    Some((item_ids, current_id, progress_secs * 1000))
}

/// Push the current queue to the active source. Silently does nothing unless
/// the source supports it and the whole queue is its own tracks.
pub fn push(ctrl: PlayerController) {
    let source = ctrl.active_source.peek().clone();
    if !source.capabilities().play_queue {
        return;
    }
    let queue = ctrl.queue.peek().clone();
    let idx = *ctrl.current_queue_index.peek();
    let progress_secs = *ctrl.current_song_progress.peek();
    let Some((item_ids, current_id, position_ms)) = save_payload(&queue, idx, progress_secs) else {
        return;
    };

    spawn(async move {
        if let Err(e) = source
            .save_play_queue(&item_ids, current_id.as_deref(), position_ms)
            .await
        {
            tracing::debug!(error = %e, "play queue push failed");
        }
    });
}

/// Fetch the active source's saved queue and, if the last write came from
/// another client, adopt it — run once at startup, after the local queue has
/// already been restored, so a fresher remote state wins over it.
pub async fn restore_if_changed_elsewhere(mut ctrl: PlayerController) {
    let source = ctrl.active_source.peek().clone();
    if !source.capabilities().play_queue {
        return;
    }
    let Ok(Some(remote)) = source.get_play_queue().await else {
        return;
    };
    if remote.tracks.is_empty() {
        return;
    }
    let changed_elsewhere = remote
        .changed_by
        .as_deref()
        .is_some_and(|by| !by.eq_ignore_ascii_case(OWN_CLIENT_NAME));
    if !changed_elsewhere {
        return;
    }

    let current_idx = remote
        .current_id
        .as_deref()
        .and_then(|id| remote.tracks.iter().position(|t| t.id.key() == id))
        .unwrap_or(0);
    let progress_secs = remote.position_ms / 1000;

    ctrl.restore_queue_state(remote.tracks, current_idx, progress_secs, Vec::new(), false);
    crate::toast::toast(&match remote.changed_by {
        Some(by) => format!("Play queue restored from {by}"),
        None => "Play queue restored from the server".to_string(),
    });
}
