use super::controls::{PlaybackControl, ProgressBarControl, VolumeControl};
use super::metadata::TrackMetadata;
use crate::lyrics_view::LyricsView;
use crate::queue_list_view::QueueListView;
use config::AppConfig;
use dioxus::prelude::*;
use player::player::Player;

#[component]
pub(crate) fn FullscreenAndroid(
    player: Signal<Player>,
    is_playing: Signal<bool>,
    mut is_fullscreen: Signal<bool>,
    config: Signal<AppConfig>,
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
    let mut active_tab = use_signal(|| 0usize);
    let tab = *active_tab.read();
    let close_text = i18n::t("close").to_string();
    let music_text = i18n::t("music").to_string();
    let up_next_text = i18n::t("up_next").to_string();
    let lyrics_text = i18n::t("lyrics").to_string();
    let tab_btn = |idx: usize, icon: &'static str, label: String| {
        let cls = if tab == idx {
            "flex-1 h-10 flex items-center justify-center text-white border-b-2 border-white"
        } else {
            "flex-1 h-10 flex items-center justify-center text-white/40 border-b-2 border-transparent"
        };
        rsx! {
            button { class: "{cls}", "aria-label": label, onclick: move |_| active_tab.set(idx),
                i { class: "{icon} text-base", "aria-hidden": "true" }
            }
        }
    };

    rsx! {
        div {
            class: "fixed inset-0 z-50 flex flex-col text-white select-none",
            style: "{background_style.read()}",

            if let Some(cover) = cover_background() {
                crate::CoverArtBackground { cover }
            }

            div {
                class: "flex items-center gap-2 px-3 pt-[env(safe-area-inset-top)] pb-1 shrink-0",
                button {
                    class: "w-10 h-10 flex items-center justify-center text-white/60 active:scale-95 transition-all shrink-0",
                    "aria-label": "{close_text}",
                    onclick: move |_| is_fullscreen.set(false),
                    i { class: "fa-solid fa-chevron-down text-xl", "aria-hidden": "true" }
                }
                div { class: "flex flex-1 items-center",
                    {tab_btn(0, "fa-solid fa-compact-disc", music_text.clone())}
                    {tab_btn(1, "fa-solid fa-list", up_next_text.clone())}
                    {tab_btn(2, "fa-solid fa-align-left", lyrics_text.clone())}
                }
            }

            if tab == 0 {
                div {
                    class: "flex-1 overflow-y-auto flex flex-col items-center justify-center px-6 pb-[calc(env(safe-area-inset-bottom)_+_1.5rem)]",
                    TrackMetadata {
                        is_fullscreen,
                        current_song_cover_url,
                        current_song_title,
                        current_song_artist,
                        current_song_album,
                        current_song_bitrate,
                    }
                    ProgressBarControl { current_song_duration, current_song_progress }
                    PlaybackControl { is_playing }
                    VolumeControl { player, config, volume, persisted_volume }
                }
            } else if tab == 1 {
                QueueListView {
                    items,
                    config,
                    current_queue_index,
                    layout: crate::queue_list_view::LayoutMode::Fullscreen,
                }
            } else {
                LyricsView {
                    lyrics,
                    current_song_progress,
                    config,
                    layout: crate::lyrics_view::LayoutMode::Fullscreen,
                }
            }
        }
    }
}
