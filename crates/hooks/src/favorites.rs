//! Favorite commands and error notifications. The daemon owns optimistic updates
//! and remote reconciliation.

use dioxus::prelude::*;

use crate::api::consume_api;

/// Toggle one track's favourite state. A no-op for an empty key.
pub fn toggle_favorite(key: String) {
    if key.trim().is_empty() {
        return;
    }
    let api = consume_api();
    spawn(async move {
        let favorite = match api.favorites().await {
            Ok(view) => !view.refs.iter().any(|existing| existing == &key),
            Err(error) => {
                tracing::warn!(%error, "could not read favorites");
                return;
            }
        };
        if let Err(error) = api.set_favorite(key.clone(), favorite).await {
            crate::toast::toast_error(&error.to_string());
        }
    });
}

/// Set every track to `on` -- the home hero's heart, favouriting a whole
/// album. Tracks already in the target state are skipped, because pushing
/// them again is at best wasted requests and at worst a rejection that would
/// revert a state which was correct.
pub fn set_favorite_many(keys: Vec<String>, on: bool) {
    if keys.is_empty() {
        return;
    }
    let api = consume_api();
    spawn(async move {
        let current = match api.favorites().await {
            Ok(view) => view.refs,
            Err(error) => {
                tracing::warn!(%error, "could not read favorites");
                return;
            }
        };
        let mut refused = false;
        for key in keys {
            if current.iter().any(|existing| existing == &key) == on {
                continue;
            }
            if let Err(error) = api.set_favorite(key.clone(), on).await {
                tracing::warn!(%error, %key, "favorite rejected");
                refused = true;
            }
        }
        if refused {
            crate::toast::toast_error("Couldn't update some favorites");
        }
    });
}

/// The heart's argument, from whatever the player is currently showing.
pub fn current(ctrl: &crate::use_player_controller::PlayerController) -> String {
    match ctrl.current_track_snapshot.read().as_ref() {
        Some(track) => track.key.clone(),
        None => String::new(),
    }
}
