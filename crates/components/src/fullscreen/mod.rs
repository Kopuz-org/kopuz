mod android;
mod background;
mod desktop;
mod lyrics;
mod metadata;
mod tabs;

use android::FullscreenAndroid;
use background::use_fullscreen_background;
use config::AppConfig;
use desktop::FullscreenDesktop;
use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;
use lyrics::use_fullscreen_lyrics;
use player::player::Player;

fn display_order_items(
    ctrl: &PlayerController,
    queue: &Signal<Vec<reader::Track>>,
) -> Vec<reader::Track> {
    let q = queue.read();
    if *ctrl.shuffle.read() {
        ctrl.shuffle_order
            .read()
            .iter()
            .filter_map(|&qi| q.get(qi).cloned())
            .collect()
    } else {
        (0..q.len()).filter_map(|qi| q.get(qi).cloned()).collect()
    }
}

#[component]
pub fn Fullscreen(
    player: Signal<Player>,
    is_playing: Signal<bool>,
    is_fullscreen: Signal<bool>,
    current_song_duration: Signal<u64>,
    current_song_progress: Signal<u64>,
    queue: Signal<Vec<reader::Track>>,
    current_queue_index: Signal<usize>,
    current_song_title: Signal<String>,
    current_song_artist: Signal<String>,
    current_song_bitrate: Signal<u16>,
    current_song_cover_url: Signal<String>,
    current_song_album: Signal<String>,
    volume: Signal<f32>,
    persisted_volume: Signal<f32>,
    palette: Signal<Option<Vec<utils::color::Color>>>,
) -> Element {
    if !*is_fullscreen.read() {
        return rsx! { div {} };
    }

    let ctrl = use_context::<PlayerController>();
    let config = use_context::<Signal<AppConfig>>();

    let lyrics = use_fullscreen_lyrics(
        current_song_title,
        current_song_artist,
        current_song_album,
        current_song_duration,
    );
    let (background_style, cover_background) =
        use_fullscreen_background(palette, current_song_cover_url);
    let items = display_order_items(&ctrl, &queue);

    if cfg!(target_os = "android") {
        rsx! {
            FullscreenAndroid {
                player,
                is_playing,
                is_fullscreen,
                config,
                current_song_duration,
                current_song_progress,
                current_song_title,
                current_song_artist,
                current_song_album,
                current_song_bitrate,
                current_song_cover_url,
                current_queue_index,
                items,
                lyrics,
                volume,
                persisted_volume,
                background_style,
                cover_background,
            }
        }
    } else {
        rsx! {
            FullscreenDesktop {
                player,
                is_playing,
                is_fullscreen,
                config,
                current_song_duration,
                current_song_progress,
                current_song_title,
                current_song_artist,
                current_song_album,
                current_song_bitrate,
                current_song_cover_url,
                current_queue_index,
                items,
                lyrics,
                volume,
                persisted_volume,
                background_style,
                cover_background,
            }
        }
    }
}
