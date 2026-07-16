//! Favorite toggling on the active source, optimistically.
//!
//! The heart flips immediately: the local state is written
//! ([`record_favorite`](server::source::MediaSource::record_favorite)) and shown,
//! then the change is pushed to the remote in the background
//! ([`push_favorite`](server::source::MediaSource::push_favorite)); if the push
//! is rejected the local state is reverted and a toast explains why, so a
//! snapping-back heart doesn't read as a broken UI.

use dioxus::prelude::*;
use reader::Track;
use server::source::ActiveSource;

use crate::db_reactivity::{Generations, Table};

/// Toggle `track`'s favorite state on the active source, optimistically (write +
/// show immediately, push in the background, revert + toast if the remote
/// rejects it). A no-op for an empty key.
pub fn toggle_favorite(track: Option<Track>) {
    let Some(track) = track else { return };
    if track.id.key().trim().is_empty() {
        return;
    }
    let source = consume_context::<Signal<ActiveSource>>().peek().clone();
    let gens = consume_context::<Generations>();

    spawn(async move {
        let key = track.id.key().to_string();
        let new_fav = !source.is_favorite(&key).await;

        // Optimistic: write locally and reflect it on the heart right away.
        if let Err(e) = source.record_favorite(&track, new_fav).await {
            tracing::warn!(error = %e, track = %track.id.uid(), "favorite: local write failed");
            return;
        }
        bump_favorites(gens);

        // Push in the background; revert the local state if the remote rejects it.
        if let Err(e) = source.push_favorite(&key, new_fav).await {
            tracing::warn!(error = %e, track = %track.id.uid(), "favorite push rejected; reverting");
            let _ = source.record_favorite(&track, !new_fav).await;
            bump_favorites(gens);
            crate::toast::toast_error(&favorite_error(&track));
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
    let source = consume_context::<Signal<ActiveSource>>().peek().clone();
    let gens = consume_context::<Generations>();

    spawn(async move {
        // Optimistic: record every track locally, then show them all. Tracks
        // already in the target state are skipped — pushing them again is at
        // best wasted requests, at worst a remote rejection (e.g. deleting a
        // like that doesn't exist) that would revert a state that was correct.
        let mut recorded = Vec::new();
        for track in tracks {
            let key = track.id.key().to_string();
            if key.trim().is_empty() {
                continue;
            }
            if source.is_favorite(&key).await == on {
                continue;
            }
            if source.record_favorite(&track, on).await.is_ok() {
                recorded.push(track);
            }
        }
        if recorded.is_empty() {
            return;
        }
        bump_favorites(gens);

        // Push each; revert the ones the remote rejects.
        let mut reverted = false;
        for track in recorded {
            let key = track.id.key().to_string();
            if let Err(e) = source.push_favorite(&key, on).await {
                tracing::warn!(error = %e, track = %track.id.uid(), "favorite push rejected; reverting");
                let _ = source.record_favorite(&track, !on).await;
                reverted = true;
            }
        }
        if reverted {
            bump_favorites(gens);
            crate::toast::toast_error("Couldn't update some favorites");
        }
    });
}

/// A short "the server rejected it" notice, so a reverted heart doesn't read as
/// a broken UI. Names the service when the track has one.
fn favorite_error(track: &Track) -> String {
    match track.id.service() {
        Some(service) => format!("Couldn't update favorite on {}", service.display_name()),
        None => "Couldn't update favorite".to_string(),
    }
}

fn bump_favorites(gens: Generations) {
    gens.bump(Table::Favorites);
    gens.bump(Table::Tracks);
}
