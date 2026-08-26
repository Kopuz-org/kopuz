//! Favorite toggling on the active source, optimistically.
//!
//! The heart flips immediately: the local state is written
//! ([`record_favorite`](server::source::MediaSource::record_favorite)) and shown,
//! then the change is pushed to the remote in the background
//! ([`push_favorite`](server::source::MediaSource::push_favorite)); if the push
//! is rejected the local state is reverted and a toast explains why, so a
//! snapping-back heart doesn't read as a broken UI.

use std::sync::Arc;

use api::KopuzApi;
use dioxus::prelude::*;
use reader::Track;

/// Toggle `track`'s favorite state on the active source, optimistically (write +
/// show immediately, push in the background, revert + toast if the remote
/// rejects it). A no-op for an empty key.
pub fn toggle_favorite(track: Option<Track>) {
    let Some(track) = track else { return };
    if track.id.key().trim().is_empty() {
        return;
    }
    let api = consume_context::<Arc<dyn KopuzApi>>();

    spawn(async move {
        let key = track.id.key().to_string();
        let favorite = api
            .favorites()
            .await
            .map(|favorites| favorites.refs.contains(&key))
            .unwrap_or(false);
        if let Err(error) = api.set_favorite(key, !favorite).await {
            tracing::warn!(%error, track = %track.id.uid(), "favorite update failed");
            let msg = match track.id.service() {
                Some(service) => format!("Couldn't update favorite on {}", service.display_name()),
                None => "Couldn't update favorite".to_string(),
            };
            crate::toast::toast_error(&msg);
        }
    });
}

/// Set every track in `tracks` to `on` on the active source (the home-hero heart,
/// favoriting a whole album). Optimistic: all are recorded and shown, then
/// pushed; any the remote rejects are reverted.
pub fn set_favorite_many(tracks: Vec<Track>, on: bool) {
    if tracks.is_empty() {
        return;
    }
    let api = consume_context::<Arc<dyn KopuzApi>>();

    spawn(async move {
        let current: std::collections::HashSet<String> = api
            .favorites()
            .await
            .map(|favorites| favorites.refs.into_iter().collect())
            .unwrap_or_default();
        let mut failed = false;
        for track in tracks {
            let key = track.id.key().to_string();
            if key.trim().is_empty() {
                continue;
            }
            if current.contains(&key) == on {
                continue;
            }
            if let Err(error) = api.set_favorite(key, on).await {
                tracing::warn!(%error, track = %track.id.uid(), "favorite update failed");
                failed = true;
            }
        }
        if failed {
            crate::toast::toast_error("Couldn't update some favorites");
        }
    });
}
