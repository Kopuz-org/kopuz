use crate::NavigationController;
use config::PlayerBarPosition;
use dioxus::prelude::*;
use hooks::use_player_controller::{LoopMode, PlayerController};
use player::player::Player;
use reader::{FavoritesStore, Library};

use crate::shared::{fmt_time, get_favorite, toggle_favorite};

#[component]
pub fn BottombarModern(
    library: Signal<Library>,
    favorites_store: Signal<FavoritesStore>,
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
    let mut is_dragging = use_signal(|| false);
    let mut drag_progress = use_signal(|| 0u64);

    let initial_volume = *volume.read();
    let mut is_muted = use_signal(move || initial_volume <= f32::EPSILON);
    let mut volume_before_mute = use_signal(move || {
        if initial_volume > f32::EPSILON {
            initial_volume
        } else {
            0.5f32
        }
    });

    let display_progress = if *is_dragging.read() {
        *drag_progress.read()
    } else {
        *current_song_progress.read()
    };

    let progress_percent = if *current_song_duration.read() > 0 {
        (display_progress as f64 / *current_song_duration.read() as f64) * 100.0
    } else {
        0.0
    };

    let volume_percent = *volume.read() * 100.0;
    let mut ctrl = use_context::<PlayerController>();
    let nav_ctrl = use_context::<NavigationController>();

    let current_track_snapshot = ctrl.current_track_snapshot.read().clone();
    let is_favorite = get_favorite(current_track_snapshot.as_ref(), &favorites_store);
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

    let is_radio = *current_song_duration.read() == u64::MAX;

    rsx! {
        div {
            class: "h-16 bg-black/70 backdrop-blur-xl {border_class} px-4 flex items-center gap-3 select-none shrink-0",

            div {
                class: "flex items-center gap-2 shrink-0",
                button {
                    class: format!("w-7 h-7 flex items-center justify-center transition-all active:scale-90 {}",
                        if *ctrl.shuffle.read() { "text-white" } else { "text-slate-500 hover:text-white" }
                    ),
                    title: if *ctrl.shuffle.read() { i18n::t("shuffle_on").to_string() } else { i18n::t("shuffle_off").to_string() },
                    onclick: move |_| ctrl.toggle_shuffle(),
                    i { class: "fa-solid fa-shuffle text-[11px]" }
                }
                button {
                    class: "w-7 h-7 flex items-center justify-center text-slate-400 hover:text-white transition-all active:scale-90",
                    onclick: move |_| ctrl.play_prev(),
                    i { class: "fa-solid fa-backward-step text-sm" }
                }
                button {
                    class: "w-8 h-8 flex items-center justify-center rounded-full bg-white/10 hover:bg-white/20 text-white transition-all active:scale-95",
                    onclick: move |_| ctrl.toggle(),
                    i { class: if *is_playing.read() { "fa-solid fa-pause text-xs" } else { "fa-solid fa-play text-xs ml-0.5" } }
                }
                button {
                    class: "w-7 h-7 flex items-center justify-center text-slate-400 hover:text-white transition-all active:scale-90",
                    onclick: move |_| ctrl.play_next(),
                    i { class: "fa-solid fa-forward-step text-sm" }
                }
                button {
                    class: format!("w-7 h-7 flex items-center justify-center transition-all active:scale-90 relative {}",
                        match *ctrl.loop_mode.read() {
                            LoopMode::None => "text-slate-500 hover:text-white",
                            _ => "text-white",
                        }
                    ),
                    title: match *ctrl.loop_mode.read() {
                        LoopMode::None => i18n::t("repeat_off").to_string(),
                        LoopMode::Queue => i18n::t("repeat_queue").to_string(),
                        LoopMode::Track => i18n::t("repeat_track").to_string(),
                    },
                    onclick: move |_| ctrl.toggle_loop(),
                    i { class: "fa-solid fa-repeat text-[11px]" }
                    if let LoopMode::Track = *ctrl.loop_mode.read() {
                        span { class: "absolute -bottom-1.5 left-1/2 -translate-x-1/2 text-[8px] font-bold leading-none", "1" }
                    }
                }
            }

            div { class: "w-px h-5 bg-white/10 shrink-0" }

            div {
                class: "w-11 h-11 rounded overflow-hidden bg-white/5 shrink-0 flex items-center justify-center",
                if current_song_cover_url.read().is_empty() {
                    i { class: "fa-solid fa-music text-white/20 text-xs" }
                } else {
                    img { src: "{current_song_cover_url}", class: "w-full h-full object-cover" }
                }
            }

            div {
                class: "flex flex-col flex-1 min-w-0 justify-center gap-0.5",
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
                div {
                    class: "flex items-center gap-1.5 w-full",
                    span { class: "text-[9px] text-slate-600 font-mono shrink-0 w-7 text-right", "{fmt_time(display_progress)}" }
                    div {
                        class: format!("flex-1 h-[3px] bg-white/10 rounded-full relative {}", if is_radio { "" } else { "group cursor-pointer" }),
                        div {
                            class: "absolute top-0 left-0 h-full bg-white/60 group-hover:bg-white rounded-full transition-colors pointer-events-none",
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
                            class: format!("absolute top-0 left-0 w-full h-full opacity-0 z-10 {}", if is_radio { "pointer-events-none" } else { "cursor-pointer" }),
                            disabled: is_radio,
                            onchange: move |evt| {
                                if let Ok(val) = evt.value().parse::<f64>().map(|v| v as u64) {
                                    player.write().seek(std::time::Duration::from_secs(val));
                                    current_song_progress.set(val);
                                    drag_progress.set(val);
                                    is_dragging.set(false);
                                }
                            },
                            oninput: move |evt| {
                                if let Ok(val) = evt.value().parse::<f64>().map(|v| v as u64) {
                                    is_dragging.set(true);
                                    drag_progress.set(val);
                                }
                            }
                        }
                    }
                    span { class: "text-[9px] text-slate-600 font-mono shrink-0 w-7", "{fmt_time(*current_song_duration.read())}" }
                }
            }

            div { class: "w-px h-5 bg-white/10 shrink-0" }

            div {
                class: "flex items-center gap-2 shrink-0",
                button {
                    class: "{heart_class} w-7 h-7 flex items-center justify-center",
                    title: if is_favorite { i18n::t("remove_from_favorites").to_string() } else { i18n::t("add_to_favorites").to_string() },
                    onclick: move |_| toggle_favorite(ctrl.current_track_snapshot.read().clone(), favorites_store, config),
                    i { class: "{heart_icon} text-xs" }
                }
                div {
                    class: "flex items-center gap-1.5",
                    button {
                        class: "w-6 h-6 flex items-center justify-center text-slate-500 hover:text-white transition-colors",
                        onclick: move |_| {
                            let muted = *is_muted.read();
                            if muted {
                                let vol = *volume_before_mute.read();
                                player.write().set_volume(vol);
                                volume.set(vol);
                                persisted_volume.set(vol);
                                is_muted.set(false);
                            } else {
                                volume_before_mute.set(*volume.read());
                                player.write().set_volume(0.0);
                                volume.set(0.0);
                                persisted_volume.set(0.0);
                                is_muted.set(true);
                            }
                        },
                        i { class: if *is_muted.read() { "fa-solid fa-volume-xmark text-[10px]" } else { "fa-solid fa-volume-high text-[10px]" } }
                    }
                    div {
                        class: "w-20 h-[3px] bg-white/10 rounded-full group/vol cursor-pointer relative",
                        onwheel: move |evt| {
                            evt.stop_propagation();
                            let dy = evt.delta().strip_units().y;
                            if dy.abs() < f64::EPSILON {
                                return;
                            }
                            let step = config.read().volume_scroll_step.max(0.0);
                            let dir = if dy < 0.0 { 1.0 } else { -1.0 };
                            let current = *volume.read();
                            let new_val = (current + dir * step).clamp(0.0, 1.0);
                            player.write().set_volume(new_val);
                            volume.set(new_val);
                            persisted_volume.set(new_val);
                            is_muted.set(new_val <= f32::EPSILON);
                            if new_val > f32::EPSILON {
                                volume_before_mute.set(new_val);
                            }
                        },
                        div {
                            class: "absolute top-0 left-0 h-full bg-white/60 group-hover/vol:bg-white rounded-full transition-colors pointer-events-none",
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
                            class: "absolute top-0 left-0 w-full h-full opacity-0 cursor-pointer z-10",
                            onchange: move |evt| {
                                if let Ok(val) = evt.value().parse::<f32>() {
                                    persisted_volume.set(val);
                                    is_muted.set(val == 0.0);
                                }
                            },
                            oninput: move |evt| {
                                if let Ok(val) = evt.value().parse::<f32>() {
                                    player.write().set_volume(val);
                                    volume.set(val);
                                    is_muted.set(val == 0.0);
                                    if val > f32::EPSILON {
                                        volume_before_mute.set(val);
                                    }
                                }
                            }
                        }
                    }
                }
                button {
                    class: "w-7 h-7 flex items-center justify-center text-slate-500 hover:text-white transition-colors",
                    onclick: move |_| { let c = *is_rightbar_open.read(); is_rightbar_open.set(!c); },
                    i { class: "fa-solid fa-list text-[10px]" }
                }
                button {
                    class: "w-7 h-7 flex items-center justify-center text-slate-500 hover:text-white transition-colors",
                    onclick: move |_| is_fullscreen.set(true),
                    i { class: "fa-solid fa-up-right-and-down-left-from-center text-[10px]" }
                }
            }
        }
    }
}
