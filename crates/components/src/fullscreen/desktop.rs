use super::controls::{ProgressBarControl, VolumeControl};
use super::metadata::TrackMetadata;
use super::tabs::Tabs;
use crate::player_controls::{TransportButtons, TransportVariant};
use crate::titlebar::Titlebar;
use config::AppConfig;
use dioxus::prelude::*;
use player::player::Player;

#[component]
pub(crate) fn FullscreenDesktop(
    player: Signal<Player>,
    is_playing: Signal<bool>,
    mut is_fullscreen: Signal<bool>,
    mut config: Signal<AppConfig>,
    current_song_duration: Signal<u64>,
    current_song_progress: Signal<u64>,
    current_song_title: Signal<String>,
    current_song_artist: Signal<String>,
    current_song_album: Signal<String>,
    current_song_bitrate: Signal<u16>,
    current_song_cover_url: Signal<String>,
    current_queue_index: Signal<usize>,
    items: Vec<reader::Track>,
    lyrics: Signal<Option<Option<utils::lyrics::Lyrics>>>,
    volume: Signal<f32>,
    persisted_volume: Signal<f32>,
    background_style: Memo<String>,
    cover_background: Memo<Option<String>>,
) -> Element {
    let use_player_bar = config.read().fullscreen_use_player_bar;
    let tabs_collapsed = config.read().fullscreen_tabs_collapsed;

    rsx! {
        div {
            class: "fixed inset-0 z-50 flex flex-col text-white select-none",
            style: "{background_style.read()}",

            if let Some(cover) = cover_background() {
                crate::CoverArtBackground { cover }
            }

            if cfg!(any(target_os = "linux", target_os = "windows")) {
                div { dir: "ltr", Titlebar {} }
            }

            div {
                class: if use_player_bar {
                    match config.read().player_bar_position {
                        config::PlayerBarPosition::Top => "flex flex-1 overflow-hidden pt-24",
                        config::PlayerBarPosition::Bottom => "flex flex-1 overflow-hidden pb-24",
                    }
                } else {
                    "flex flex-1 overflow-hidden"
                },

                div {
                    class: "flex flex-col items-center justify-center p-6 pt-16 lg:p-10 lg:pt-16 relative flex-shrink-0 overflow-hidden min-h-0",
                    style: if tabs_collapsed { "width: 100%;" } else { "width: 50%;" },

                    button {
                        class: "absolute top-8 left-8 text-white/30 hover:text-white transition-colors z-10",
                        onclick: move |_| is_fullscreen.set(false),
                        i { class: "fa-solid fa-chevron-down text-2xl" }
                    }

                    TrackMetadata {
                        is_fullscreen,
                        current_song_cover_url,
                        current_song_title,
                        current_song_artist,
                        current_song_album,
                        current_song_bitrate,
                    }

                    if !use_player_bar {
                        ProgressBarControl {
                            current_song_duration,
                            current_song_progress,
                        }

                        TransportButtons { is_playing, variant: TransportVariant::Fullscreen }

                        VolumeControl { player, config, volume, persisted_volume }
                    }
                }

                if !tabs_collapsed {
                    Tabs {
                        config,
                        items,
                        current_queue_index,
                        lyrics,
                        current_song_progress,
                    }
                }
            }

            if tabs_collapsed {
                button {
                    class: "absolute right-4 top-1/2 -translate-y-1/2 z-10 w-9 h-9 rounded-full bg-black/40 hover:bg-black/60 backdrop-blur text-white/70 hover:text-white flex items-center justify-center shadow-lg ring-1 ring-white/10 transition-colors",
                    "aria-label": i18n::t("show_side_panel").to_string(),
                    title: i18n::t("show_side_panel").to_string(),
                    onclick: move |_| config.write().fullscreen_tabs_collapsed = false,
                    i { class: "fa-solid fa-chevron-left text-sm" }
                }
            }
        }
    }
}
