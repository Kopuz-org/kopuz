use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;

/// The words for what is playing, for the fullscreen view. The daemon finds
/// them; this only says which track and what to show when there are none.
pub(crate) fn use_fullscreen_lyrics() -> Signal<Option<Option<utils::lyrics::Lyrics>>> {
    let ctrl = use_context::<PlayerController>();
    let track_key = use_memo(move || {
        ctrl.current_track_snapshot
            .read()
            .as_ref()
            .map(|track| track.uid.clone())
            .unwrap_or_default()
    });
    hooks::lyrics::use_lyrics(track_key, i18n::t("lyrics_not_found").to_string())
}
