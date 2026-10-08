//! Telling the source what not to play again.
//!
//! The negative counterpart to a favourite: it is a signal to the source's
//! recommender, not a library edit, so the daemon owns the call and what is
//! left here is what the player does next.

use dioxus::prelude::*;

use crate::api::consume_api;
use crate::use_player_controller::PlayerController;

/// "Don't recommend this": tell the source to stop surfacing whatever is
/// playing, then move on. The skip is unconditional -- it is local, it is the
/// half the person can see, and leaving them on a song they just rejected
/// because a request failed would be the worse answer; the toast says what
/// didn't land.
pub fn dont_recommend(mut ctrl: PlayerController) {
    let key = match ctrl.current_track_snapshot.read().as_ref() {
        Some(track) => track.key.clone(),
        None => return,
    };
    if key.trim().is_empty() {
        return;
    }
    let api = consume_api();
    spawn(async move {
        if let Err(error) = api.dont_recommend(key).await {
            crate::toast::toast_error(&error.to_string());
        }
        ctrl.play_next();
    });
}
