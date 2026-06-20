use config::UiStyle;
use dioxus::prelude::*;
use player::player::Player;

use crate::modern::bottombar::BottombarModern;
use crate::normal::bottombar::BottombarNormal;
use crate::queue_drag::{install_native_artwork_drag_prevention, set_queue_drag_enabled};

#[component]
pub fn Bottombar(
    config: Signal<config::AppConfig>,
    player: Signal<Player>,
    is_playing: Signal<bool>,
    is_fullscreen: Signal<bool>,
    current_song_duration: Signal<u64>,
    current_song_progress: Signal<u64>,
    queue: Signal<Vec<reader::models::Track>>,
    current_queue_index: Signal<usize>,
    current_song_title: Signal<String>,
    current_song_artist: Signal<String>,
    current_song_cover_url: Signal<String>,
    volume: Signal<f32>,
    persisted_volume: Signal<f32>,
    is_rightbar_open: Signal<bool>,
) -> Element {
    use_effect(move || {
        install_native_artwork_drag_prevention();
    });

    let c = *is_rightbar_open.read();
    set_queue_drag_enabled(c);

    match config.read().ui_style {
        UiStyle::Normal => rsx! {
            BottombarNormal {
                config, player, is_playing, is_fullscreen,
                current_song_duration, current_song_progress, queue, current_queue_index,
                current_song_title, current_song_artist, current_song_cover_url,
                volume, persisted_volume, is_rightbar_open,
            }
        },
        UiStyle::Modern => rsx! {
            BottombarModern {
                config, player, is_playing, is_fullscreen,
                current_song_duration, current_song_progress, queue, current_queue_index,
                current_song_title, current_song_artist, current_song_cover_url,
                volume, persisted_volume, is_rightbar_open,
            }
        },
    }
}
