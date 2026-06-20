use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;

/// The play/pause overlay button on an album card. Reflects the player: it shows
/// pause while *this* album is the one playing, and clicking it then toggles
/// play/pause instead of restarting. Any other album → play it. Styling is the
/// caller's (cards differ in size/colour); only the icon + behaviour are shared.
#[component]
pub fn AlbumPlayButton(
    /// The card's album id; `None` (e.g. a recently-played track with no album)
    /// just plays nothing and never shows the playing state.
    album_id: Option<String>,
    on_play_album: EventHandler<String>,
    /// The button wrapper's classes (positioning, size, background).
    class: String,
    /// Optional inline style (some cards set the background via a CSS var).
    #[props(default)]
    style: String,
    /// Icon size/colour suffix, e.g. `"text-sm"` or `"text-white text-xs"`.
    icon_extra: String,
) -> Element {
    let mut ctrl = use_context::<PlayerController>();
    let is_current = match (&album_id, ctrl.current_track()) {
        (Some(id), Some(track)) => &track.album_id == id,
        _ => false,
    };
    let is_playing = is_current && *ctrl.is_playing.read();
    let icon = if is_playing {
        format!("fa-solid fa-pause {icon_extra}")
    } else {
        format!("fa-solid fa-play ml-0.5 {icon_extra}")
    };
    rsx! {
        div {
            class: "{class}",
            style: "{style}",
            onclick: move |evt| {
                evt.stop_propagation();
                if is_current {
                    ctrl.toggle();
                } else if let Some(id) = album_id.clone() {
                    on_play_album.call(id);
                }
            },
            i { class: "{icon}" }
        }
    }
}
