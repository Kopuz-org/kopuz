use crate::NavigationController;
use crate::player_controls::{ControlsVariant, SeekSlider, TransportButtons, VolumeSlider};
use config::PlayerBarPosition;
use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;
use player::player::Player;

use hooks::favorites::toggle_favorite;

#[component]
pub fn BottombarNormal(
    mut config: Signal<config::AppConfig>,
    mut player: Signal<Player>,
    mut is_playing: Signal<bool>,
    mut is_fullscreen: Signal<bool>,
    mut current_song_duration: Signal<u64>,
    mut current_song_progress: Signal<u64>,
    queue: Signal<Vec<reader::models::Track>>,
    mut current_queue_index: Signal<usize>,
    mut current_song_title: Signal<String>,
    mut current_song_artist: Signal<String>,
    mut current_song_cover_url: Signal<String>,
    mut volume: Signal<f32>,
    mut persisted_volume: Signal<f32>,
    mut is_rightbar_open: Signal<bool>,
) -> Element {
    let mut ctrl = use_context::<PlayerController>();
    let active_source = use_context::<Signal<::server::source::ActiveSource>>();
    let nav_ctrl = use_context::<NavigationController>();
    let fav_track = use_memo(move || ctrl.current_track_snapshot.read().clone());
    let is_fav = hooks::use_db_queries::use_track_is_favorite(fav_track);
    let crate::CompactMode(mut compact_mode) = use_context::<crate::CompactMode>();

    if cfg!(target_os = "android") {
        let progress_percent = if *current_song_duration.read() > 0 {
            (*current_song_progress.read() as f64 / *current_song_duration.read() as f64) * 100.0
        } else {
            0.0
        };
        let cover = current_song_cover_url.read().clone();
        return rsx! {
            div {
                class: "shrink-0 mx-2 mb-[env(safe-area-inset-bottom)] h-[68px] bg-[#121212]/95 backdrop-blur-3xl border border-white/10 rounded-[24px] flex items-center px-3 gap-3 relative overflow-hidden shadow-[0_12px_40px_rgba(0,0,0,0.8)]",
                onclick: move |_| is_fullscreen.set(true),
                div { class: "absolute top-0 left-0 h-[2px] bg-white/10 w-full",
                    div { class: "h-full bg-white transition-all duration-300", style: "width: {progress_percent}%" }
                }
                div { class: "w-11 h-11 bg-white/5 rounded-xl shrink-0 overflow-hidden flex items-center justify-center",
                    if cover.is_empty() {
                        i { class: "fa-solid fa-music text-white/20" }
                    } else {
                        img { src: "{cover}", class: "w-full h-full object-cover" }
                    }
                }
                div { class: "flex-1 min-w-0 flex flex-col justify-center gap-0.5",
                    span { class: "text-[13px] font-bold text-white truncate leading-tight", "{current_song_title}" }
                    span { class: "text-[11px] font-medium text-white/60 truncate leading-tight", "{current_song_artist}" }
                }
                div { class: "flex items-center gap-1 pr-1",
                    button {
                        class: "w-12 h-12 flex items-center justify-center text-white text-xl active:scale-90 transition-transform",
                        onclick: move |evt| { evt.stop_propagation(); ctrl.toggle(); },
                        i { class: if *is_playing.read() { "fa-solid fa-pause" } else { "fa-solid fa-play ml-1" } }
                    }
                    button {
                        class: "w-12 h-12 flex items-center justify-center text-white text-lg active:scale-90 transition-transform",
                        onclick: move |evt| { evt.stop_propagation(); ctrl.play_next(); },
                        i { class: "fa-solid fa-forward-step" }
                    }
                }
            }
        };
    }

    let current_track_snapshot = ctrl.current_track_snapshot.read().clone();
    let is_favorite = is_fav();
    let heart_class = if is_favorite {
        "ml-2 w-9 h-9 rounded-full flex items-center justify-center text-red-400 hover:text-red-300 hover:bg-white/10 transition-colors active:scale-95"
    } else {
        "ml-2 w-9 h-9 rounded-full flex items-center justify-center text-slate-400 hover:text-red-400 hover:bg-white/10 transition-colors active:scale-95"
    };
    let heart_icon = if is_favorite {
        "fa-solid fa-heart"
    } else {
        "fa-regular fa-heart"
    };

    let position = config.read().player_bar_position;
    let border_class = match position {
        PlayerBarPosition::Bottom => "border-t border-white/5",
        PlayerBarPosition::Top => "border-b border-white/5",
    };

    let bar_as_fullscreen = *is_fullscreen.read() && config.read().fullscreen_use_player_bar;
    let lift_class = if bar_as_fullscreen {
        "relative z-[60]"
    } else {
        ""
    };
    let bg_class =
        if config.read().cover_art_background || !config.read().custom_background_path.is_empty() {
            "bg-black/40"
        } else {
            "bg-black/60"
        };

    rsx! {
        div {
            class: "h-24 {bg_class} {border_class} {lift_class} px-4 flex items-center justify-between select-text shrink-0",

            div {
                class: "flex items-center gap-4 w-1/4",
                if !bar_as_fullscreen {
                    div {
                        class: "w-14 h-14 bg-white/5 rounded-md flex-shrink-0 overflow-hidden",
                        if current_song_cover_url.read().is_empty() {
                            div {
                                class: "w-full h-full flex items-center justify-center",
                                style: "font-size: 1.5em;",
                                i { class: "fa-solid fa-music text-white/20" }
                            }
                        } else {
                            img { src: "{current_song_cover_url}", class: "w-full h-full object-cover" }
                        }
                    }
                    div {
                        class: "flex flex-col min-w-0",
                        span {
                            class: "text-sm font-bold text-white/90 truncate hover:underline cursor-pointer",
                            onclick: move |_| {
                                let album_id = current_track_snapshot
                                    .as_ref()
                                    .map(|track| track.album_id.clone())
                                    .unwrap_or_default();
                                nav_ctrl.navigate_to_album(album_id);
                            },
                            "{current_song_title}"
                        }
                        span {
                            class: "text-xs text-slate-400 truncate hover:text-white/70 hover:underline cursor-pointer",
                            onclick: move |_| {
                                let artist = current_song_artist.read().clone();
                                nav_ctrl.navigate_to_artist(artist);
                            },
                            "{current_song_artist}"
                        }
                    }
                }
                button {
                    class: "{heart_class}",
                    title: if is_favorite { i18n::t("remove_from_favorites").to_string() } else { i18n::t("add_to_favorites").to_string() },
                    onclick: move |_| toggle_favorite(ctrl.current_track_snapshot.read().clone()),
                    i { class: "{heart_icon}" }
                }
            }

            div {
                class: "flex flex-col items-center max-w-[40%] w-full gap-2",
                TransportButtons { is_playing, variant: ControlsVariant::Bar }
                SeekSlider { current_song_duration, current_song_progress, variant: ControlsVariant::Bar }
            }

            div {
                class: "flex items-center justify-end gap-4 w-1/4",
                VolumeSlider { player, config, volume, persisted_volume, variant: ControlsVariant::Bar }
                button {
                    class: "w-9 h-9 rounded-full flex items-center justify-center text-slate-400 hover:text-white hover:bg-white/10 transition-colors active:scale-95",
                    onclick: move |_| { let c = *is_rightbar_open.read(); is_rightbar_open.set(!c); },
                    i { class: "fa-solid fa-list text-xs" }
                }
                button {
                    class: "w-9 h-9 rounded-full flex items-center justify-center text-slate-400 hover:text-white hover:bg-white/10 transition-colors active:scale-95",
                    title: i18n::t("share_musicbrainz").to_string(),
                    onclick: move |_| {
                        if let Some(t) = ctrl.current_track_snapshot.read().clone() {
                            let src = active_source.peek().clone();
                            crate::track_row::share_track(t, src);
                        }
                    },
                    i { class: "fa-solid fa-share-nodes text-xs" }
                }
                if cfg!(not(target_os = "android")) {
                    button {
                        class: "w-9 h-9 rounded-full flex items-center justify-center text-slate-400 hover:text-white hover:bg-white/10 transition-colors active:scale-95",
                        title: i18n::t("mini_player").to_string(),
                        onclick: move |_| { let c = *compact_mode.read(); compact_mode.set(!c); },
                        i { class: "fa-solid fa-compress text-xs" }
                    }
                }
                if !bar_as_fullscreen {
                    button {
                        class: "w-9 h-9 rounded-full flex items-center justify-center text-slate-400 hover:text-white hover:bg-white/10 transition-colors active:scale-95",
                        onclick: move |_| is_fullscreen.set(true),
                        i { class: "fa-solid fa-up-right-and-down-left-from-center text-xs" }
                    }
                }
            }
        }
    }
}
