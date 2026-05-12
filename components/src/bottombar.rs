use config::MusicService;
use dioxus::prelude::*;
use hooks::use_player_controller::{LoopMode, PlayerController};
use player::player::Player;
use reader::{FavoritesStore, Library};

#[component]
pub fn Bottombar(
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
    let apple_ui = config.read().apple_music_ui;
    
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

    let format_time = |seconds: u64| {
        let minutes = seconds / 60;
        let seconds = seconds % 60;
        format!("{}:{:02}", minutes, seconds)
    };

    let progress_percent = if *current_song_duration.read() > 0 {
        (display_progress as f64 / *current_song_duration.read() as f64) * 100.0
    } else {
        0.0
    };

    let volume_percent = *volume.read() * 100.0;

    let mut ctrl = use_context::<PlayerController>();

    let is_favorite = {
        let q = queue.read();
        let idx = *current_queue_index.read();
        if let Some(track) = q.get(idx) {
            let path_str = track.path.to_string_lossy();
            if path_str.starts_with("jellyfin:")
                || path_str.starts_with("subsonic:")
                || path_str.starts_with("custom:")
            {
                let parts: Vec<&str> = path_str.split(':').collect();
                if parts.len() >= 2 && !parts[1].trim().is_empty() {
                    favorites_store.read().is_jellyfin_favorite(parts[1])
                } else {
                    false
                }
            } else {
                favorites_store.read().is_local_favorite(&track.path)
            }
        } else {
            false
        }
    };

    let heart_class = if is_favorite {
        "ml-1 text-red-400 hover:text-red-300 transition-colors"
    } else {
        "ml-1 text-slate-500 hover:text-red-400 transition-colors"
    };

    let heart_icon = if is_favorite {
        "fa-solid fa-heart"
    } else {
        "fa-regular fa-heart"
    };

    let toggle_favorite = move |_| {
        let q = queue.read();
        let idx = *current_queue_index.read();
        if let Some(track) = q.get(idx).cloned() {
            drop(q);
            let path_str = track.path.to_string_lossy().to_string();
            let is_server_item = path_str.starts_with("jellyfin:")
                || path_str.starts_with("subsonic:")
                || path_str.starts_with("custom:");
            if is_server_item {
                let parts: Vec<String> = path_str.split(':').map(|s| s.to_string()).collect();
                if parts.len() >= 2 && !parts[1].trim().is_empty() {
                    let item_id = parts[1].clone();
                    let currently_fav = favorites_store.read().is_jellyfin_favorite(&item_id);
                    let new_fav = !currently_fav;
                    favorites_store.write().set_jellyfin(item_id.clone(), new_fav);
                    spawn(async move {
                        let server_config = {
                            let conf = config.peek();
                            conf.server.as_ref().and_then(|server| {
                                if let (Some(token), Some(user_id)) = (&server.access_token, &server.user_id) {
                                    Some((server.service, server.url.clone(), token.clone(), user_id.clone(), conf.device_id.clone()))
                                } else { None }
                            })
                        };
                        if let Some((service, url, token, user_id, device_id)) = server_config {
                            let result = match service {
                                MusicService::Jellyfin => {
                                    let remote = server::jellyfin::JellyfinClient::new(&url, Some(&token), &device_id, Some(&user_id));
                                    if new_fav { remote.mark_favorite(&item_id).await } else { remote.unmark_favorite(&item_id).await }
                                }
                                MusicService::Subsonic | MusicService::Custom => {
                                    let remote = server::subsonic::SubsonicClient::new(&url, &user_id, &token);
                                    if new_fav { remote.star(&item_id).await } else { remote.unstar(&item_id).await }
                                }
                            };
                            if let Err(e) = result {
                                eprintln!("Failed to sync favorite to server: {e}");
                                favorites_store.write().set_jellyfin(item_id, !new_fav);
                            }
                        }
                    });
                }
            } else {
                favorites_store.write().toggle_local(track.path.clone());
            }
        }
    };

    let toggle_mute = move |_| {
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
    };

    rsx! {
        if apple_ui {
            // ── Cider-style compact top control bar ──────────────────────────────
            div {
                class: "h-[52px] bg-[#111111]/95 backdrop-blur-2xl border-b border-white/[0.07] px-4 flex items-center gap-3 select-text shrink-0",

                // Left: transport controls
                div {
                    class: "flex items-center gap-4 shrink-0",
                    button {
                        class: format!("transition-all active:scale-95", ),
                        style: if *ctrl.shuffle.read() { "color: #ff3b5c;" } else { "color: rgba(255,255,255,0.4);" },
                        onclick: move |_| ctrl.toggle_shuffle(),
                        i { class: "fa-solid fa-shuffle text-[13px]" }
                    }
                    button {
                        class: "text-white/50 hover:text-white transition-all active:scale-90",
                        onclick: move |_| ctrl.play_prev(),
                        i { class: "fa-solid fa-backward-step text-[16px]" }
                    }
                    button {
                        class: "w-8 h-8 bg-white rounded-full flex items-center justify-center text-black hover:scale-105 active:scale-95 transition-all",
                        onclick: move |_| ctrl.toggle(),
                        i { class: if *is_playing.read() { "fa-solid fa-pause text-[11px]" } else { "fa-solid fa-play text-[11px] ml-0.5" } }
                    }
                    button {
                        class: "text-white/50 hover:text-white transition-all active:scale-90",
                        onclick: move |_| ctrl.play_next(),
                        i { class: "fa-solid fa-forward-step text-[16px]" }
                    }
                    button {
                        class: "relative transition-all active:scale-95",
                        style: match *ctrl.loop_mode.read() {
                            LoopMode::None => "color: rgba(255,255,255,0.4);",
                            _ => "color: #ff3b5c;",
                        },
                        onclick: move |_| ctrl.toggle_loop(),
                        i { class: "fa-solid fa-repeat text-[13px]" }
                        if *ctrl.loop_mode.read() == LoopMode::Track {
                            span { class: "absolute -bottom-2 left-1/2 -translate-x-1/2 text-[8px] font-bold leading-none", "1" }
                        }
                    }
                }

                // Center: album art + song info + progress bar
                div {
                    class: "flex-1 flex items-center gap-3 min-w-0 mx-2",
                    div {
                        class: "w-9 h-9 rounded-md overflow-hidden bg-white/[0.06] shrink-0",
                        if current_song_cover_url.read().is_empty() {
                            div { class: "w-full h-full flex items-center justify-center",
                                i { class: "fa-solid fa-music text-white/20 text-sm" }
                            }
                        } else {
                            img { src: "{current_song_cover_url}", class: "w-full h-full object-cover" }
                        }
                    }
                    div {
                        class: "flex flex-col min-w-0 w-[130px] shrink-0",
                        span { class: "text-[12px] font-semibold text-white truncate leading-tight", "{current_song_title}" }
                        span { class: "text-[10px] text-white/50 truncate", "{current_song_artist}" }
                    }
                    span { class: "text-[10px] text-white/40 font-mono shrink-0", "{format_time(display_progress)}" }
                    div {
                        class: "flex-1 h-[3px] bg-white/10 rounded-full group cursor-pointer relative",
                        div {
                            class: "absolute top-0 left-0 h-full bg-white/70 group-hover:bg-white rounded-full transition-colors pointer-events-none",
                            style: "width: {progress_percent}%",
                            div { class: "absolute -right-1.5 -top-[3px] w-[9px] h-[9px] bg-white rounded-full opacity-0 group-hover:opacity-100 transition-opacity" }
                        }
                        input {
                            r#type: "range", min: "0",
                            max: "{*current_song_duration.read()}",
                            value: "{display_progress}",
                            class: "absolute top-0 left-0 w-full h-full opacity-0 cursor-pointer z-10",
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
                    span { class: "text-[10px] text-white/40 font-mono shrink-0", "{format_time(*current_song_duration.read())}" }
                }

                // Right: heart + volume + extras
                div {
                    class: "flex items-center gap-3 shrink-0",
                    button {
                        class: "{heart_class}",
                        onclick: toggle_favorite,
                        i { class: "{heart_icon} text-[13px]" }
                    }
                    button {
                        class: "text-white/40 hover:text-white transition-colors",
                        onclick: toggle_mute,
                        i { class: if *is_muted.read() { "fa-solid fa-volume-xmark text-[12px]" } else { "fa-solid fa-volume-high text-[12px]" } }
                    }
                    div {
                        class: "w-20 h-[3px] bg-white/10 rounded-full group/vol cursor-pointer relative",
                        div {
                            class: "absolute top-0 left-0 h-full bg-white/60 group-hover/vol:bg-white rounded-full pointer-events-none",
                            style: "width: {volume_percent}%",
                            div { class: "absolute -right-1.5 -top-[3px] w-[9px] h-[9px] bg-white rounded-full opacity-0 group-hover/vol:opacity-100" }
                        }
                        input {
                            r#type: "range", min: "0", max: "1", step: "0.01",
                            value: "{*volume.read()}",
                            class: "absolute top-0 left-0 w-full h-full opacity-0 cursor-pointer z-10",
                            onchange: move |evt| { if let Ok(val) = evt.value().parse::<f32>() { persisted_volume.set(val); is_muted.set(val == 0.0); } },
                            oninput: move |evt| {
                                if let Ok(val) = evt.value().parse::<f32>() {
                                    player.write().set_volume(val);
                                    volume.set(val);
                                    is_muted.set(val == 0.0);
                                    if val > f32::EPSILON { volume_before_mute.set(val); }
                                }
                            }
                        }
                    }
                    button {
                        class: "text-white/40 hover:text-white transition-colors",
                        onclick: move |_| { let c = *is_rightbar_open.read(); is_rightbar_open.set(!c); },
                        i { class: "fa-solid fa-list text-[12px]" }
                    }
                    button {
                        class: "text-white/40 hover:text-white transition-colors",
                        onclick: move |_| is_fullscreen.set(true),
                        i { class: "fa-solid fa-up-right-and-down-left-from-center text-[12px]" }
                    }
                }
            }
        } else {
            // ── Original bottom bar ───────────────────────────────────────────────
            div {
                class: "h-24 bg-black/60 border-t border-white/5 px-4 flex items-center justify-between select-text shrink-0",

                div {
                    class: "flex items-center gap-4 w-1/4",
                    div {
                        class: "w-14 h-14 bg-white/5 rounded-md flex-shrink-0 overflow-hidden",
                        if current_song_cover_url.read().is_empty() {
                            div { class: "w-full h-full flex items-center justify-center", style: "font-size: 1.5em;",
                                i { class: "fa-solid fa-music text-white/20" }
                            }
                        } else {
                            img { src: "{current_song_cover_url}", class: "w-full h-full object-cover" }
                        }
                    }
                    div {
                        class: "flex flex-col min-w-0",
                        span { class: "text-sm font-bold text-white/90 truncate hover:underline cursor-pointer", "{current_song_title}" }
                        span { class: "text-xs text-slate-400 truncate hover:text-white/70 cursor-pointer", "{current_song_artist}" }
                    }
                    button {
                        class: "{heart_class}",
                        onclick: toggle_favorite,
                        i { class: "{heart_icon}" }
                    }
                }

                div {
                    class: "flex flex-col items-center max-w-[40%] w-full gap-2",
                    div {
                        class: "flex items-center gap-6",
                        button {
                            class: format!("{} transition-all active:scale-95 relative", if *ctrl.shuffle.read() { "text-white" } else { "text-slate-400 hover:text-white" }),
                            onclick: move |_| ctrl.toggle_shuffle(),
                            i { class: "fa-solid fa-shuffle text-sm" }
                        }
                        button {
                            class: "text-slate-400 hover:text-white transition-all active:scale-90",
                            onclick: move |_| ctrl.play_prev(),
                            i { class: "fa-solid fa-backward-step text-xl" }
                        }
                        button {
                            class: "w-10 h-10 bg-white rounded-full flex items-center justify-center text-black hover:scale-105 active:scale-95 transition-all",
                            onclick: move |_| ctrl.toggle(),
                            i { class: if *is_playing.read() { "fa-solid fa-pause text-lg" } else { "fa-solid fa-play text-lg ml-0.5" } }
                        }
                        button {
                            class: "text-slate-400 hover:text-white transition-all active:scale-90",
                            onclick: move |_| ctrl.play_next(),
                            i { class: "fa-solid fa-forward-step text-xl" }
                        }
                        button {
                            class: format!("{} transition-all active:scale-95 relative",
                                match *ctrl.loop_mode.read() {
                                    LoopMode::None => "text-slate-400 hover:text-white",
                                    LoopMode::Queue | LoopMode::Track => "text-white",
                                }
                            ),
                            onclick: move |_| ctrl.toggle_loop(),
                            i { class: "fa-solid fa-repeat text-sm" }
                            if *ctrl.loop_mode.read() == LoopMode::Track {
                                span { class: "absolute -bottom-2.5 left-1/2 -translate-x-1/2 text-[9px] font-bold text-white leading-none", "1" }
                            }
                        }
                    }
                    div {
                        class: "flex items-center gap-2 w-full",
                        span { class: "text-[10px] text-slate-500 w-8 text-right font-mono", "{format_time(display_progress)}" }
                        div {
                            class: "flex-1 h-1 bg-white/10 rounded-full group cursor-pointer relative",
                            div {
                                class: "absolute top-0 left-0 h-full bg-white group-hover:bg-green-500 rounded-full transition-colors pointer-events-none",
                                style: "width: {progress_percent}%",
                                div { class: "absolute -right-1.5 -top-1 w-3 h-3 bg-white rounded-full opacity-0 group-hover:opacity-100 transition-opacity" }
                            }
                            input {
                                r#type: "range", min: "0",
                                max: "{*current_song_duration.read()}",
                                value: "{display_progress}",
                                class: "absolute top-0 left-0 w-full h-full opacity-0 cursor-pointer z-10",
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
                        span { class: "text-[10px] text-slate-500 w-8 font-mono", "{format_time(*current_song_duration.read())}" }
                    }
                }

                div {
                    class: "flex items-center justify-end gap-4 w-1/4",
                    div {
                        class: "flex items-center gap-2 group",
                        button {
                            class: "text-slate-400 hover:text-white transition-colors",
                            onclick: toggle_mute,
                            i { class: if *is_muted.read() { "fa-solid fa-volume-xmark text-xs" } else { "fa-solid fa-volume-high text-xs" } }
                        }
                        div {
                            class: "w-24 h-1 bg-white/10 rounded-full group/vol cursor-pointer relative",
                            div {
                                class: "absolute top-0 left-0 h-full bg-white group-hover/vol:bg-green-500 rounded-full transition-colors pointer-events-none",
                                style: "width: {volume_percent}%",
                                div { class: "absolute -right-1.5 -top-1 w-3 h-3 bg-white rounded-full opacity-0 group-hover/vol:opacity-100 transition-opacity" }
                            }
                            input {
                                r#type: "range", min: "0", max: "1", step: "0.01",
                                value: "{*volume.read()}",
                                class: "absolute top-0 left-0 w-full h-full opacity-0 cursor-pointer z-10",
                                onchange: move |evt| { if let Ok(val) = evt.value().parse::<f32>() { persisted_volume.set(val); is_muted.set(val == 0.0); } },
                                oninput: move |evt| {
                                    if let Ok(val) = evt.value().parse::<f32>() {
                                        player.write().set_volume(val);
                                        volume.set(val);
                                        is_muted.set(val == 0.0);
                                        if val > f32::EPSILON { volume_before_mute.set(val); }
                                    }
                                }
                            }
                        }
                    }
                    button {
                        class: "text-slate-400 hover:text-white",
                        onclick: move |_| { let c = *is_rightbar_open.read(); is_rightbar_open.set(!c); },
                        i { class: "fa-solid fa-list text-xs" }
                    }
                    button {
                        class: "text-slate-400 hover:text-white",
                        onclick: move |_| is_fullscreen.set(true),
                        i { class: "fa-solid fa-up-right-and-down-left-from-center text-xs" }
                    }
                }
            }
        }
    }
}
