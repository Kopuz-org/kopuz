use crate::NavigationController;
use crate::player_controls::{TransportButtons, TransportVariant, use_seek_drag, use_volume_mute};
use config::PlayerBarPosition;
use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;
use player::player::Player;

use crate::shared::fmt_time;
use hooks::favorites::toggle_favorite;

#[component]
pub fn BottombarVaxry(
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
    let seek = use_seek_drag(current_song_duration, current_song_progress);
    let vol = use_volume_mute(player, config, volume, persisted_volume);
    let display_progress = seek.display_progress;
    let progress_percent = seek.progress_percent;
    let seek_commit = seek.on_commit;
    let seek_input = seek.on_input;
    let volume_percent = vol.volume_percent;
    let is_muted = vol.is_muted;
    let toggle_mute = vol.toggle_mute;
    let volume_wheel = vol.on_wheel;
    let volume_commit = vol.on_commit;
    let volume_input = vol.on_input;
    let mut ctrl = use_context::<PlayerController>();
    let active_source = use_context::<Signal<::server::source::ActiveSource>>();
    let nav_ctrl = use_context::<NavigationController>();
    let fav_track = use_memo(move || ctrl.current_track_snapshot.read().clone());
    let is_fav = hooks::use_db_queries::use_track_is_favorite(fav_track);
    let crate::CompactMode(mut compact_mode) = use_context::<crate::CompactMode>();
    if cfg!(target_os = "android") {
        let pct = if *current_song_duration.read() > 0 {
            (*current_song_progress.read() as f64 / *current_song_duration.read() as f64) * 100.0
        } else {
            0.0
        };
        let cover = current_song_cover_url.read().clone();
        let fav = is_fav();
        return rsx! {
            div {
                class: "shrink-0 h-[68px] bg-black/85 backdrop-blur-2xl border-t border-white/10 flex items-center px-3 gap-3 relative overflow-hidden mb-[env(safe-area-inset-bottom)]",
                onclick: move |_| is_fullscreen.set(true),
                div { class: "absolute top-0 left-0 h-[2px] bg-white/10 w-full",
                    div { class: "h-full bg-white/80 transition-all duration-300", style: "width: {pct}%" }
                }
                div { class: "w-11 h-11 bg-white/5 rounded shrink-0 overflow-hidden flex items-center justify-center",
                    if cover.is_empty() {
                        i { class: "fa-solid fa-music text-white/20" }
                    } else {
                        img { src: "{cover}", class: "w-full h-full object-cover" }
                    }
                }
                div { class: "flex-1 min-w-0 flex flex-col justify-center gap-0.5",
                    span { class: "text-[13px] font-semibold text-white/90 truncate leading-tight", "{current_song_title}" }
                    span { class: "text-[11px] text-slate-400 truncate leading-tight", "{current_song_artist}" }
                }
                div { class: "flex items-center gap-0.5 pr-1",
                    button {
                        class: if fav { "w-10 h-10 flex items-center justify-center text-red-400 active:scale-90 transition-transform" } else { "w-10 h-10 flex items-center justify-center text-slate-400 active:scale-90 transition-transform" },
                        onclick: move |evt| { evt.stop_propagation(); toggle_favorite(ctrl.current_track_snapshot.read().clone()); },
                        i { class: if fav { "fa-solid fa-heart text-sm" } else { "fa-regular fa-heart text-sm" } }
                    }
                    button {
                        class: "w-11 h-11 flex items-center justify-center text-white text-xl active:scale-90 transition-transform",
                        onclick: move |evt| { evt.stop_propagation(); ctrl.toggle(); },
                        i { class: if *is_playing.read() { "fa-solid fa-pause" } else { "fa-solid fa-play ml-1" } }
                    }
                    button {
                        class: "w-11 h-11 flex items-center justify-center text-white text-lg active:scale-90 transition-transform",
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
        "text-red-400 hover:text-red-300 transition-colors"
    } else {
        "text-slate-500 hover:text-red-400 transition-colors"
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
            "bg-black/70 backdrop-blur-xl"
        };

    let is_radio = *current_song_duration.read() == u64::MAX;

    rsx! {
        div {
            class: "h-16 {bg_class} {border_class} {lift_class} px-4 flex items-center gap-3 select-none shrink-0",

            div {
                class: "shrink-0",
                TransportButtons { is_playing, variant: TransportVariant::Bar }
            }

            div { class: "w-px h-5 bg-white/10 shrink-0" }

            if !bar_as_fullscreen {
                div {
                    class: "w-11 h-11 rounded overflow-hidden bg-white/5 shrink-0 flex items-center justify-center",
                    if current_song_cover_url.read().is_empty() {
                        i { class: "fa-solid fa-music text-white/20 text-xs" }
                    } else {
                        img { src: "{current_song_cover_url}", class: "w-full h-full object-cover" }
                    }
                }
            }

            div {
                class: "flex flex-col flex-1 min-w-0 justify-center gap-0.5",
                if !bar_as_fullscreen {
                    div {
                        class: "flex items-baseline gap-1.5 min-w-0",
                        span {
                            class: "text-xs font-semibold text-white/90 truncate hover:underline cursor-pointer shrink-0 max-w-[40%]",
                            onclick: move |_| {
                                let album_id = current_track_snapshot
                                    .as_ref()
                                    .map(|track| track.album_id.clone())
                                    .unwrap_or_default();
                                nav_ctrl.navigate_to_album(album_id);
                            },
                            "{current_song_title}"
                        }
                        span { class: "text-white/20 text-[10px] shrink-0", "—" }
                        span {
                            class: "text-[11px] text-slate-400 truncate min-w-0 cursor-pointer hover:underline hover:text-slate-300",
                            onclick: move |_| {
                                let artist = current_song_artist.read().clone();
                                nav_ctrl.navigate_to_artist(artist);
                            },
                            "{current_song_artist}"
                        }
                    }
                }
                div {
                    class: "flex items-center gap-1.5 w-full",
                    span { class: "text-[9px] text-slate-600 font-mono shrink-0 w-7 text-right", "{fmt_time(display_progress)}" }
                    div {
                        class: format!("flex-1 h-[3px] bg-white/10 rounded-full relative {}", if is_radio { "" } else { "group cursor-pointer" }),
                        div {
                            class: "absolute top-0 left-0 h-full bg-white/90 rounded-full pointer-events-none",
                            style: "width: {progress_percent}%",
                        }
                        div {
                            class: "absolute top-1/2 -translate-y-1/2 w-2.5 h-2.5 bg-white rounded-full opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none -translate-x-1/2",
                            style: "left: {progress_percent}%",
                        }
                        input {
                            r#type: "range",
                            min: "0",
                            max: "{*current_song_duration.read()}",
                            value: "{display_progress}",
                            class: format!("slider-hit absolute top-0 left-0 w-full h-full opacity-0 z-10 {}", if is_radio { "pointer-events-none" } else { "cursor-pointer" }),
                            disabled: is_radio,
                            onchange: move |evt| seek_commit.call(evt),
                            oninput: move |evt| seek_input.call(evt),
                        }
                    }
                    span { class: "text-[9px] text-slate-600 font-mono shrink-0 w-7", "{fmt_time(*current_song_duration.read())}" }
                }
            }

            div { class: "w-px h-5 bg-white/10 shrink-0" }

            div {
                class: "flex items-center gap-2 shrink-0",
                button {
                    class: "{heart_class} w-9 h-9 rounded-full flex items-center justify-center hover:bg-white/10 active:scale-95",
                    title: if is_favorite { i18n::t("remove_from_favorites").to_string() } else { i18n::t("add_to_favorites").to_string() },
                    onclick: move |_| toggle_favorite(ctrl.current_track_snapshot.read().clone()),
                    i { class: "{heart_icon} text-xs" }
                }
                div {
                    class: "flex items-center gap-1.5",
                    button {
                        class: "w-9 h-9 rounded-full flex items-center justify-center text-slate-500 hover:text-white hover:bg-white/10 transition-colors active:scale-95",
                        onclick: move |_| toggle_mute.call(()),
                        i { class: if is_muted { "fa-solid fa-volume-xmark text-[10px]" } else { "fa-solid fa-volume-high text-[10px]" } }
                    }
                    div {
                        class: "w-20 h-[3px] bg-white/10 rounded-full group/vol cursor-pointer relative",
                        onwheel: move |evt| volume_wheel.call(evt),
                        div {
                            class: "absolute top-0 left-0 h-full bg-white/90 rounded-full pointer-events-none",
                            style: "width: {volume_percent}%",
                        }
                        div {
                            class: "absolute top-1/2 -translate-y-1/2 w-2.5 h-2.5 bg-white rounded-full opacity-0 group-hover/vol:opacity-100 transition-opacity pointer-events-none -translate-x-1/2",
                            style: "left: {volume_percent}%",
                        }
                        input {
                            r#type: "range",
                            min: "0",
                            max: "1",
                            step: "0.01",
                            value: "{*volume.read()}",
                            class: "slider-hit absolute top-0 left-0 w-full h-full opacity-0 cursor-pointer z-10",
                            onchange: move |evt| volume_commit.call(evt),
                            oninput: move |evt| volume_input.call(evt),
                        }
                    }
                }
                button {
                    class: "w-9 h-9 rounded-full flex items-center justify-center text-slate-500 hover:text-white hover:bg-white/10 transition-colors active:scale-95",
                    onclick: move |_| { let c = *is_rightbar_open.read(); is_rightbar_open.set(!c); },
                    i { class: "fa-solid fa-list text-[10px]" }
                }
                button {
                    class: "w-9 h-9 rounded-full flex items-center justify-center text-slate-500 hover:text-white hover:bg-white/10 transition-colors active:scale-95",
                    title: i18n::t("share_musicbrainz").to_string(),
                    onclick: move |_| {
                        if let Some(t) = ctrl.current_track_snapshot.read().clone() {
                            let src = active_source.peek().clone();
                            crate::track_row::share_track(t, src);
                        }
                    },
                    i { class: "fa-solid fa-share-nodes text-[10px]" }
                }
                if cfg!(not(target_os = "android")) {
                    button {
                        class: "w-9 h-9 rounded-full flex items-center justify-center text-slate-500 hover:text-white hover:bg-white/10 transition-colors active:scale-95",
                        title: i18n::t("mini_player").to_string(),
                        onclick: move |_| { let c = *compact_mode.read(); compact_mode.set(!c); },
                        i { class: "fa-solid fa-compress text-[10px]" }
                    }
                }
                if !bar_as_fullscreen {
                    button {
                        class: "w-9 h-9 rounded-full flex items-center justify-center text-slate-500 hover:text-white hover:bg-white/10 transition-colors active:scale-95",
                        onclick: move |_| is_fullscreen.set(true),
                        i { class: "fa-solid fa-up-right-and-down-left-from-center text-[10px]" }
                    }
                }
            }
        }
    }
}
