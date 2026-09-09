//! "Start radio" as one action, shared by every surface that offers it.
//!
//! A track seed and a playlist seed are the same operation from the UI's side:
//! name an id, get a generated queue, play it. Keeping the label, the icon, the
//! capability gate and the notices in one place is what stops the track row and
//! the playlist surfaces from drifting apart.
//!
//! Building a mix is a remote round trip that can take tens of seconds, so it
//! announces itself: without the opening notice a click looks like it did
//! nothing, and without the failure one a timeout is indistinguishable from
//! success.

use dioxus::prelude::*;
use hooks::PlayerController;
use hooks::use_player_controller::RadioNotices;

pub const RADIO_ICON: &str = "fa-solid fa-tower-broadcast";

pub fn radio_label() -> String {
    i18n::t("start_radio").to_string()
}

fn notices() -> RadioNotices {
    RadioNotices {
        starting: i18n::t("radio_starting").to_string(),
        failed: i18n::t("radio_failed").to_string(),
    }
}

/// The `on_start_radio` handler for a track row: `Some` when the active source
/// can seed a mix from a track, else `None` so the row hides the action.
///
/// Reads context with `consume_context`, never a `use_*` hook: call sites
/// invoke this once per visible row, so a hook here would register a
/// per-row-count number of hooks and panic the parent on rules-of-hooks when
/// the row count changes.
pub fn track_radio_handler(key: String) -> Option<EventHandler<()>> {
    let mut ctrl = consume_context::<PlayerController>();
    let caps = consume_context::<Signal<api::SourceCapabilities>>();
    let supported = caps.read().track_radio;
    supported.then(|| EventHandler::new(move |_| ctrl.play_track_radio(key.clone(), notices())))
}

/// The playlist counterpart. Gated on its own flag: a Subsonic server can seed
/// a mix from a song but not from a playlist, so sharing the track flag put an
/// action on playlist cards that could only ever fail.
pub fn playlist_radio_handler(playlist_id: String) -> Option<EventHandler<()>> {
    let mut ctrl = consume_context::<PlayerController>();
    let caps = consume_context::<Signal<api::SourceCapabilities>>();
    let supported = caps.read().playlist_radio;
    supported.then(|| {
        EventHandler::new(move |_| ctrl.play_playlist_radio(playlist_id.clone(), notices()))
    })
}
