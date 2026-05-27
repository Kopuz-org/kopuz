use crate::NavigationController;
use crate::constants::*;
use crate::dots_menu::{DotsMenu, MenuAction};
use crate::queue_drag::{
    clear_dragged_queue_track, handle_select_click, is_queue_drag_enabled, set_dragged_queue_track,
    set_dragged_queue_tracks,
};
use config::{AppConfig, UiStyle};
use dioxus::prelude::*;
use hooks::PlayerController;
use reader::models::Track;

#[component]
pub fn TrackRow(
    track: Track,
    cover_url: Option<utils::CoverUrl>,
    on_click_menu: EventHandler<()>,
    is_menu_open: bool,
    #[props(default = false)] is_album: bool,
    on_add_to_playlist: EventHandler<()>,
    on_queue: Option<EventHandler<()>>,
    on_close_menu: EventHandler<()>,
    on_play: EventHandler<()>,
    on_delete: EventHandler<()>,
    on_remove_from_playlist: Option<EventHandler<()>>,
    #[props(default = false)] is_selection_mode: bool,
    #[props(default = false)] is_selected: bool,
    #[props(default = false)] hide_delete: bool,
    on_select: Option<EventHandler<bool>>,
    on_long_press: Option<EventHandler<()>>,
    on_download: Option<EventHandler<()>>,
    #[props(default = false)] is_downloaded: bool,
    #[props(default = false)] is_downloading: bool,
    #[props(default = false)] is_currently_playing: bool,
    #[props(default = Vec::new())] selected_queue_tracks: Vec<Track>,
    #[props(default = None)] row_num: Option<usize>,
) -> Element {
    let config = use_context::<Signal<AppConfig>>();
    let mut ctrl = use_context::<PlayerController>();
    let nav_ctrl = use_context::<NavigationController>();
    let is_modern = config.read().ui_style == UiStyle::Modern;
    let show_selection_highlight = is_selection_mode && is_selected;
    let selection_shadow = if show_selection_highlight {
        "inset 0 0 0 9999px rgba(255,255,255,0.07)"
    } else {
        "none"
    };
    let drag_track_mouse = track.clone();
    let drag_track_normal_mouse = track.clone();
    let drag_selected_tracks_mouse = selected_queue_tracks.clone();
    let drag_selected_tracks_normal_mouse = selected_queue_tracks.clone();
    let play_next_track_mouse = track.clone();
    let play_next_track_normal = track.clone();
    let drag_cover_url = cover_url.as_ref().map(|url| url.as_ref().to_string());
    let drag_cover_url_normal = drag_cover_url.clone();
    let mut pending_queue_drag = use_signal(|| None::<(f64, f64)>);
    let mut pending_queue_drag_normal = use_signal(|| None::<(f64, f64)>);
    const QUEUE_DRAG_THRESHOLD_PX: f64 = 6.0;
    let play_next_text = i18n::t("play_next").to_string();
    let add_to_queue_text = i18n::t("add_to_queue").to_string();
    let add_to_playlist_text = i18n::t("add_to_playlist").to_string();
    let remove_from_playlist_text = i18n::t("remove_from_playlist").to_string();
    let delete_song_text = i18n::t("delete").to_string();

    let mut actions = Vec::new();

    let has_queue = on_queue.is_some();
    if has_queue {
        actions.push(MenuAction::new(
            play_next_text.as_str(),
            "fa-solid fa-forward-step",
        ));
        actions.push(MenuAction::new(
            add_to_queue_text.as_str(),
            "fa-solid fa-list-ul",
        ));
    }

    actions.push(MenuAction::new(
        add_to_playlist_text.as_str(),
        "fa-solid fa-plus",
    ));

    let has_remove = on_remove_from_playlist.is_some();
    if has_remove {
        actions.push(MenuAction::new(
            remove_from_playlist_text.as_str(),
            "fa-solid fa-minus",
        ));
    }

    let has_download = on_download.is_some();
    if has_download {
        let (dl_label, dl_icon) = if is_downloading {
            ("Downloading...", "fa-solid fa-spinner fa-spin")
        } else if is_downloaded {
            ("Remove Download", "fa-solid fa-trash-can")
        } else {
            ("Download Offline", "fa-solid fa-download")
        };
        let mut action = MenuAction::new(dl_label, dl_icon);
        if is_downloaded {
            action = action.destructive();
        }
        actions.push(action);
    }

    if !hide_delete {
        actions.push(MenuAction::new(delete_song_text.as_str(), "fa-solid fa-trash").destructive());
    }

    let play_next_idx = if has_queue { Some(0) } else { None };
    let add_to_queue_idx = if has_queue { Some(1) } else { None };
    let add_to_playlist_idx = if has_queue { 2 } else { 0 };
    let remove_action_idx = if has_remove {
        Some(add_to_playlist_idx + 1)
    } else {
        None
    };
    let download_action_idx = if has_download {
        add_to_playlist_idx + 1 + usize::from(has_remove)
    } else {
        0
    };
    let delete_action_idx = if has_download {
        download_action_idx + 1
    } else {
        add_to_playlist_idx + 1 + usize::from(has_remove)
    };

    let mut long_press_task = use_signal(|| None);
    let mut long_press_occurred = use_signal(|| false);

    let mut start_long_press = move || {
        if is_selection_mode {
            return;
        }
        long_press_occurred.set(false);
        if let Some(handler) = on_long_press {
            let mut occurred = long_press_occurred;
            let task = spawn(async move {
                utils::sleep(std::time::Duration::from_millis(600)).await;
                occurred.set(true);
                handler.call(());
            });
            long_press_task.set(Some(task));
        }
    };

    let mut cancel_long_press = move || {
        if let Some(task) = long_press_task.write().take() {
            task.cancel();
        }
    };

    let fmt_dur = |s: u64| format!("{}:{:02}", s / 60, s % 60);
    let duration_str = fmt_dur(track.duration);

    let columns_modern = if is_album {
        COLUMNS_MODERN_ALBUM
    } else {
        COLUMNS_MODERN
    };

    if is_modern {
        return rsx! {
            div {
                class: "track-row-draggable grid px-2 py-1.5 rounded-lg mx-1 group cursor-grab active:cursor-grabbing transition-colors hover:bg-white/5 select-none",
                style: if is_currently_playing {
                    format!("grid-template-columns: {columns_modern}; background: color-mix(in oklab, var(--color-indigo-500) 12%, transparent); box-shadow: {selection_shadow};")
                } else {
                    format!("grid-template-columns: {columns_modern}; box-shadow: {selection_shadow};")
                },
                onclick: move |evt| {
                    evt.stop_propagation();
                    if *long_press_occurred.read() {
                        long_press_occurred.set(false);
                        return;
                    }
                    if is_selection_mode {
                        handle_select_click(is_selected, is_selection_mode, on_select);
                    }
                },
                draggable: "false",
                ondoubleclick: move |_| { if !is_selection_mode { on_play.call(()); } },
                onmousedown: move |evt| {
                    if is_queue_drag_enabled() && (!is_selection_mode || is_selected) {
                        let coords = evt.client_coordinates();
                        pending_queue_drag.set(Some((coords.x, coords.y)));
                    }
                    start_long_press();
                },
                onmousemove: move |evt| {
                    let drag_start = *pending_queue_drag.read();
                    if let Some((start_x, start_y)) = drag_start {
                        let coords = evt.client_coordinates();
                        let dx = coords.x - start_x;
                        let dy = coords.y - start_y;
                        if dx.hypot(dy) >= QUEUE_DRAG_THRESHOLD_PX {
                            pending_queue_drag.set(None);
                            if is_selection_mode && !drag_selected_tracks_mouse.is_empty() {
                                set_dragged_queue_tracks(
                                    drag_selected_tracks_mouse.clone(),
                                    coords.x,
                                    coords.y,
                                );
                            } else {
                                set_dragged_queue_track(
                                    drag_track_mouse.clone(),
                                    drag_cover_url.clone(),
                                    coords.x,
                                    coords.y,
                                );
                            }
                        }
                    }
                },
                onmouseup: move |_| {
                    pending_queue_drag.set(None);
                    cancel_long_press();
                    clear_dragged_queue_track();
                },
                onmouseleave: move |_| cancel_long_press(),
                ontouchstart: move |_| start_long_press(),
                ontouchend: move |_| cancel_long_press(),
                oncontextmenu: move |evt| {
                    evt.prevent_default();
                    if !is_selection_mode { on_click_menu.call(()); }
                },

                div { class: "flex items-center h-8",
                    if is_currently_playing && !is_selection_mode {
                        i {
                            class: "fa-solid fa-volume-high text-xs",
                            style: "color: var(--color-indigo-500);"
                        }
                    } else if on_select.is_some() && is_selection_mode {
                        button {
                            class: if is_selected {
                                "w-4 h-4 rounded border border-indigo-400 bg-indigo-500 text-white flex items-center justify-center transition-colors"
                            } else {
                                "w-4 h-4 rounded border border-white/20 bg-white/5 hover:border-white/50 transition-colors"
                            },
                            onclick: move |evt| {
                                evt.stop_propagation();
                                handle_select_click(is_selected, is_selection_mode, on_select);
                            },
                            if is_selected { i { class: "fa-solid fa-check", style: "font-size: 9px;" } }
                        }
                    } else {
                        if let Some(n) = row_num {
                            span {
                                class: "text-xs group-hover:hidden text-white/25",
                                "{n}"

                            }
                        }
                        button {
                            class: if row_num.is_some() {
                                "hidden group-hover:flex items-center justify-center"
                            } else {
                                "flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity"
                            },
                            onclick: move |_| on_play.call(()),
                            i { class: "fa-solid fa-play text-xs text-white/80" }
                        }
                    }
                }

                div { class: "flex items-center min-w-0 pr-3 gap-2",
                    if !is_album {
                        div {
                            class: "w-8 h-8 rounded overflow-hidden shrink-0",
                            style: "background: url('data:image/svg+xml,%3Csvg xmlns=%27http://www.w3.org/2000/svg%27 width=%27400%27 height=%27400%27 viewBox=%270 0 400 400%27%3E%3Crect width=%27400%27 height=%27400%27 fill=%27%231e1b2e%27/%3E%3Ccircle cx=%27200%27 cy=%27180%27 r=%2770%27 fill=%27none%27 stroke=%27%233d3466%27 stroke-width=%276%27/%3E%3Cpath d=%27M155 280 Q200 240 245 280%27 fill=%27none%27 stroke=%27%233d3466%27 stroke-width=%276%27 stroke-linecap=%27round%27/%3E%3C/svg%3E') center/cover no-repeat, rgba(255,255,255,0.05);",
                            if let Some(ref url) = cover_url {
                                img {
                                    src: "{url.as_ref()}",
                                    class: "w-full h-full object-cover",
                                    loading: "lazy",
                                    decoding: "async",
                                }
                            }
                        }
                    }
                    span {
                        class: "text-sm font-medium truncate cursor-pointer hover:underline",
                        style: if is_currently_playing {
                            "color: var(--color-indigo-500); font-weight: 600;"
                        } else {
                            "color: var(--color-white); opacity: 0.9;"
                        },
                        onclick: {
                            let album_id = track.album_id.clone();
                            move |evt: MouseEvent| {
                                evt.stop_propagation();
                                if !is_selection_mode {
                                    nav_ctrl.navigate_to_album(album_id.clone());
                                }
                            }
                        },
                        ondoubleclick: move |evt| evt.stop_propagation(),
                        "{track.title}"
                    }
                    if is_downloaded {
                        i {
                            class: "fa-solid fa-arrow-down-to-line text-[9px] shrink-0",
                            style: "color: var(--color-indigo-500); opacity: 0.7;"
                        }
                    }
                }

                div { class: "flex items-center min-w-0 pr-3",
                    span {
                        class: "text-sm truncate cursor-pointer hover:underline",
                        style: "color: var(--color-white); opacity: 0.45;",
                        onclick: {
                            let artist = track.artist.clone();
                            move |evt: MouseEvent| {
                                evt.stop_propagation();
                                if !is_selection_mode {
                                    nav_ctrl.navigate_to_artist(artist.clone());
                                }
                            }
                        },
                        ondoubleclick: move |evt| evt.stop_propagation(),
                        "{track.artist}"
                    }
                }

                if !is_album {
                    div { class: "flex items-center min-w-0 pr-3",
                        span {
                            class: "text-sm truncate cursor-pointer hover:underline",
                            style: "color: var(--color-white); opacity: 0.35;",
                            onclick: {
                                let album_id = track.album_id.clone();
                                move |evt: MouseEvent| {
                                    evt.stop_propagation();
                                    if !is_selection_mode {
                                        nav_ctrl.navigate_to_album(album_id.clone());
                                    }
                                }
                            },
                            ondoubleclick: move |evt| evt.stop_propagation(),
                            "{track.album}"
                        }
                    }
                }

                div { class: "flex items-center justify-end",
                    span {
                        class: "text-xs font-mono",
                        style: "color: var(--color-white); opacity: 0.3;",
                        "{duration_str}"
                    }
                }

                div { class: "flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity",
                    if !is_selection_mode {
                        DotsMenu {
                            actions,
                            is_open: is_menu_open,
                            on_open: move |_| on_click_menu.call(()),
                            on_close: move |_| on_close_menu.call(()),
                            button_class: "w-6 h-6 flex items-center justify-center rounded transition-colors hover:bg-white/10".to_string(),
                            anchor: "right".to_string(),
                            on_action: move |idx: usize| {
                                if let Some(play_next_idx) = play_next_idx
                                    && idx == play_next_idx
                                {
                                    ctrl.queue_play_next(vec![play_next_track_mouse.clone()]);
                                    on_close_menu.call(());
                                    return;
                                }
                                if let Some(queue_idx) = add_to_queue_idx
                                    && idx == queue_idx
                                {
                                    if let Some(handler) = on_queue {
                                        handler.call(());
                                        return;
                                    }
                                }
                                if idx == add_to_playlist_idx {
                                    on_add_to_playlist.call(());
                                } else if remove_action_idx == Some(idx) {
                                    if let Some(handler) = on_remove_from_playlist { handler.call(()); }
                                } else if has_download && idx == download_action_idx {
                                    if let Some(handler) = on_download { handler.call(()); }
                                } else if idx == delete_action_idx {
                                    on_delete.call(());
                                }
                            },
                        }
                    }
                }
            }
        };
    }

    let columns_normal = if is_album {
        COLUMNS_NORMAL_ALBUM
    } else {
        COLUMNS_NORMAL
    };

    // normal UI
    return rsx! {
        div {
            class: "track-row-draggable grid items-center h-14 p-2 rounded-lg hover:bg-white/5 group transition-colors relative select-none cursor-grab active:cursor-grabbing",
            style: if is_currently_playing {
                format!("grid-template-columns: {columns_normal}; column-gap: 1.5rem; background: color-mix(in oklab, var(--color-indigo-500) 12%, transparent); box-shadow: {selection_shadow};")
            } else {
                format!("grid-template-columns: {columns_normal}; column-gap: 1.5rem; box-shadow: {selection_shadow};")
            },
            draggable: "false",
            onclick: move |evt| {
                evt.stop_propagation();
                if *long_press_occurred.read() {
                    long_press_occurred.set(false);
                    return;
                }
                handle_select_click(is_selected, is_selection_mode, on_select);
            },
            ondoubleclick: move |evt| {
                evt.stop_propagation();
                if !is_selection_mode {
                    on_play.call(());
                }
            },
            onmousedown: move |evt| {
                if is_queue_drag_enabled() && (!is_selection_mode || is_selected) {
                    let coords = evt.client_coordinates();
                    pending_queue_drag_normal.set(Some((coords.x, coords.y)));
                }
                start_long_press();
            },
            onmousemove: move |evt| {
                let drag_start = *pending_queue_drag_normal.read();
                if let Some((start_x, start_y)) = drag_start {
                    let coords = evt.client_coordinates();
                    let dx = coords.x - start_x;
                    let dy = coords.y - start_y;
                    if dx.hypot(dy) >= QUEUE_DRAG_THRESHOLD_PX {
                        pending_queue_drag_normal.set(None);
                        if is_selection_mode && !drag_selected_tracks_normal_mouse.is_empty() {
                            set_dragged_queue_tracks(
                                drag_selected_tracks_normal_mouse.clone(),
                                coords.x,
                                coords.y,
                            );
                        } else {
                            set_dragged_queue_track(
                                drag_track_normal_mouse.clone(),
                                drag_cover_url_normal.clone(),
                                coords.x,
                                coords.y,
                            );
                        }
                    }
                }
            },
            onmouseup: move |_| {
                pending_queue_drag_normal.set(None);
                cancel_long_press();
                clear_dragged_queue_track();
            },
            onmouseleave: move |_| cancel_long_press(),
            ontouchstart: move |_| start_long_press(),
            ontouchend: move |_| cancel_long_press(),
            oncontextmenu: move |evt| {
                evt.prevent_default();
                if !is_selection_mode {
                    on_click_menu.call(());
                }
            },

            div { class: "flex justify-center items-center shrink-0",
                if on_select.is_some() && is_selection_mode {
                    button {
                        class: if is_selected {
                            "w-4 h-4 rounded border border-indigo-400 bg-indigo-500 text-white flex items-center justify-center transition-colors"
                        } else {
                            "w-4 h-4 rounded border border-white/20 bg-white/5 hover:border-white/50 transition-colors"
                        },
                        aria_label: if is_selected { "Deselect track" } else { "Select track" },
                        onclick: move |evt| {
                            evt.stop_propagation();
                            handle_select_click(is_selected, is_selection_mode, on_select);
                        },
                        if is_selected {
                            i { class: "fa-solid fa-check", style: "font-size: 9px;" }
                        }
                    }
                } else if is_currently_playing {
                    i { class: "fa-solid fa-volume-high text-xs", style: "color: var(--color-indigo-500);" }
                } else if let Some(n) = row_num {
                    span {
                        class: "text-xs group-hover:hidden text-white/60",
                        "{n}"
                    }
                    button {
                        class: if row_num.is_some() {
                            "hidden group-hover:flex items-center justify-center"
                        } else {
                            "flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity"
                        },
                        onclick: move |_| on_play.call(()),
                        i { class: "fa-solid fa-play text-m text-white/60" }
                    }
                }
            }

            div { class: "flex items-center min-w-0",
                if !is_album {
                    div { class: "relative w-10 h-10 bg-white/5 rounded overflow-hidden flex items-center justify-center mr-4 shrink-0",
                        i { class: "fa-solid fa-music text-white/20 absolute" }
                        if let Some(url) = cover_url {
                            div {
                                class: "absolute inset-0 bg-cover bg-center",
                                style: "background-image: url('{url.as_ref()}');"
                            }
                        }
                        if is_downloaded && !is_currently_playing {
                            div { class: "absolute bottom-0 right-0 w-3 h-3 bg-indigo-500 rounded-tl flex items-center justify-center",
                                    i { class: "fa-solid fa-check text-white", style: "font-size: 6px;" }
                            }
                        }
                    }
                }
                p {
                    class: "text-sm font-medium truncate cursor-pointer hover:underline",
                    style: if is_currently_playing {
                        "color: var(--color-indigo-500);"
                    } else {
                        "color: var(--color-white); opacity: 0.9;"
                    },
                    onclick: {
                        let album_id = track.album_id.clone();
                        move |evt: MouseEvent| {
                            evt.stop_propagation();
                            if !is_selection_mode {
                                nav_ctrl.navigate_to_album(album_id.clone());
                            }
                        }
                    },
                    ondoubleclick: move |evt| evt.stop_propagation(),
                    "{track.title}"
                }
            }

            div { class: "min-w-0",
                p {
                    class: "text-sm text-slate-500 truncate cursor-pointer hover:underline hover:text-slate-400 transition-colors",
                    style: "color: var(--color-white); opacity: 0.45;",
                    onclick: {
                        let artist = track.artist.clone();
                        move |evt: MouseEvent| {
                            evt.stop_propagation();
                            if !is_selection_mode {
                                nav_ctrl.navigate_to_artist(artist.clone());
                            }
                        }
                    },
                    ondoubleclick: move |evt| evt.stop_propagation(),
                    "{track.artist}"
                }
            }

            if !is_album {
                div { class: "min-w-0",
                      p {
                          class: "text-sm text-slate-500 truncate cursor-pointer hover:underline hover:text-slate-400 transition-colors",
                          style: "color: var(--color-white); opacity: 0.3;",
                          onclick: {
                              let album_id = track.album_id.clone();
                              move |evt: MouseEvent| {
                                  evt.stop_propagation();
                                  if !is_selection_mode {
                                      nav_ctrl.navigate_to_album(album_id.clone());
                                  }
                              }
                          },
                          ondoubleclick: move |evt| evt.stop_propagation(),
                          "{track.album}"
                      }
                }
            }

            div { class: "flex items-center justify-end",
                span { class: "text-xs font-mono text-slate-500", style: "color: var(--color-white); opacity: 0.3;", "{duration_str}" }
            }

            div { class: "flex items-center justify-end",
                if !is_selection_mode {
                    DotsMenu {
                        actions,
                        is_open: is_menu_open,
                        on_open: move |_| on_click_menu.call(()),
                        on_close: move |_| on_close_menu.call(()),
                        button_class: "opacity-0 group-hover:opacity-100 focus:opacity-100".to_string(),
                        anchor: "right".to_string(),
                        on_action: move |idx: usize| {
                            if let Some(play_next_idx) = play_next_idx
                                && idx == play_next_idx
                            {
                                ctrl.queue_play_next(vec![play_next_track_normal.clone()]);
                                on_close_menu.call(());
                                return;
                            }
                            if let Some(queue_idx) = add_to_queue_idx {
                                if idx == queue_idx {
                                    if let Some(handler) = on_queue {
                                        handler.call(());
                                    }
                                    return;
                                }
                            }

                            if idx == add_to_playlist_idx {
                                on_add_to_playlist.call(());
                            } else if remove_action_idx == Some(idx) {
                                if let Some(handler) = on_remove_from_playlist {
                                    handler.call(());
                                }
                            } else if has_download && idx == download_action_idx {
                                if let Some(handler) = on_download {
                                    handler.call(());
                                }
                            } else if idx == delete_action_idx {
                                on_delete.call(());
                            }
                        },
                    }
                }
            }
        }
    };
}
