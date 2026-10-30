use config::UiStyle;
use dioxus::prelude::*;

use crate::normal::bottombar::BottombarNormal;
use crate::queue_drag::{install_native_artwork_drag_prevention, set_queue_drag_enabled};
use crate::vaxry::bottombar::BottombarVaxry;

#[component]
pub fn Bottombar(
    settings: Signal<config::Settings>,
    is_playing: Signal<bool>,
    is_fullscreen: Signal<bool>,
    current_song_duration: Signal<u64>,
    current_song_progress: Signal<u64>,
    queue: Signal<Vec<api::TrackInfo>>,
    current_queue_index: Signal<usize>,
    current_song_title: Signal<String>,
    current_song_artist: Signal<String>,
    volume: Signal<f32>,
    is_rightbar_open: Signal<bool>,
    is_devices_open: Signal<bool>,
) -> Element {
    use_effect(move || {
        install_native_artwork_drag_prevention();
    });

    let c = *is_rightbar_open.read();
    set_queue_drag_enabled(c);

    match settings.read().ui.ui_style {
        UiStyle::Normal | UiStyle::Material3 => rsx! {
            BottombarNormal {
                settings, is_playing, is_fullscreen,
                current_song_duration, current_song_progress, queue, current_queue_index,
                current_song_title, current_song_artist,
                volume, is_rightbar_open, is_devices_open,
            }
        },
        UiStyle::Vaxry => rsx! {
            BottombarVaxry {
                settings, is_playing, is_fullscreen,
                current_song_duration, current_song_progress, queue, current_queue_index,
                current_song_title, current_song_artist,
                volume, is_rightbar_open, is_devices_open,
            }
        },
    }
}
