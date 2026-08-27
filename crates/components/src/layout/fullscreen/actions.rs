use crate::metadata_modal::MetadataModal;
use dioxus::prelude::*;
use reader::Track;

/// The fullscreen player's overflow menu: the shared track-actions menu wearing
/// this surface's chrome. Metadata stays here because the modal belongs to the
/// fullscreen overlay, not to the menu.
#[component]
pub(crate) fn TrackActions(track: Track, menu_open: Signal<bool>) -> Element {
    let mut menu_open = menu_open;
    let mut show_metadata = use_signal(|| false);

    rsx! {
        crate::track_actions::TrackActionsMenu {
            track: track.clone(),
            is_open: Some(menu_open()),
            on_open: Some(EventHandler::new(move |_| menu_open.set(true))),
            on_close: Some(EventHandler::new(move |_| menu_open.set(false))),
            button_class: "w-11 h-11 bg-white/10 text-white/70 hover:bg-white/15 hover:text-white active:scale-95".to_string(),
            anchor: "right".to_string(),
            placement: "top".to_string(),
            icon: "fa-solid fa-ellipsis".to_string(),
            playlist_overlay_class: Some("overlay".to_string()),
            on_view_metadata: Some(EventHandler::new(move |_| show_metadata.set(true))),
        }

        if *show_metadata.read() {
            MetadataModal {
                track: track.clone(),
                on_close: move |_| show_metadata.set(false),
            }
        }
    }
}
