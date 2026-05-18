use crate::NavigationController;
use crate::dots_menu::{DotsMenu, MenuAction};
use config::{AppConfig, UiStyle};
use dioxus::prelude::*;
use reader::models::Track;

fn handle_select_click(
    is_selected: bool,
    is_selection_mode: bool,
    on_select: Option<EventHandler<bool>>,
) {
    if is_selection_mode {
        if let Some(handler) = on_select {
            handler.call(!is_selected);
        }
    }
}

#[component]
pub fn TrackRow(
    track: Track,
    cover_url: Option<utils::CoverUrl>,
    on_click_menu: EventHandler<()>,
    is_menu_open: bool,
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
    #[props(default = None)] row_num: Option<usize>,
) -> Element {
    let config = use_context::<Signal<AppConfig>>();
    let nav_ctrl = use_context::<NavigationController>();
    let is_modern = config.read().ui_style == UiStyle::Modern;
    let show_selection_highlight = is_selection_mode && is_selected;
    let selection_shadow = if show_selection_highlight {
        "inset 0 0 0 9999px rgba(255,255,255,0.07)"
    } else {
        "none"
    };
    let add_to_queue_text = i18n::t("add_to_queue").to_string();
    let add_to_playlist_text = i18n::t("add_to_playlist").to_string();
    let remove_from_playlist_text = i18n::t("remove_from_playlist").to_string();
    let delete_song_text = i18n::t("delete").to_string();

    let mut actions = Vec::new();

    let has_queue = on_queue.is_some();
    if has_queue {
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

    let add_to_queue_idx = if has_queue { Some(0) } else { None };
    let add_to_playlist_idx = if has_queue { 1 } else { 0 };
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

    if is_modern {
        return rsx! {
            div {
                class: "grid px-2 py-1.5 rounded-lg mx-1 group cursor-default transition-colors hover:bg-white/5 select-none",
                style: if is_currently_playing {
                    format!("grid-template-columns: 40px 1fr 180px 180px 56px 40px; background: color-mix(in oklab, var(--color-indigo-500) 12%, transparent); box-shadow: {selection_shadow};")
                } else {
                    format!("grid-template-columns: 40px 1fr 180px 180px 56px 40px; box-shadow: {selection_shadow};")
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
                ondoubleclick: move |_| { if !is_selection_mode { on_play.call(()); } },
                onmousedown: move |_| start_long_press(),
                onmouseup: move |_| cancel_long_press(),
                onmouseleave: move |_| cancel_long_press(),
                ontouchstart: move |_| start_long_press(),
                ontouchend: move |_| cancel_long_press(),
                oncontextmenu: move |evt| {
                    evt.prevent_default();
                    if !is_selection_mode { on_click_menu.call(()); }
                },

                div { class: "flex items-center",
                    if is_currently_playing {
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
                    div { class: "w-8 h-8 rounded bg-white/5 overflow-hidden shrink-0 flex items-center justify-center relative",
                        i { class: "fa-solid fa-music text-white/20 absolute", style: "font-size: 10px;" }
                        if let Some(ref url) = cover_url {
                            div {
                                class: "absolute inset-0 bg-cover bg-center",
                                style: "background-image: url('{url.as_ref()}');"
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

                div { class: "flex items-center justify-end pr-2",
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
                                if let Some(queue_idx) = add_to_queue_idx {
                                    if idx == queue_idx {
                                        if let Some(handler) = on_queue { handler.call(()); }
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

    // normal UI
    return rsx! {
        div {
            class: "grid items-center p-2 rounded-lg hover:bg-white/5 group transition-colors relative select-none",
            style: if is_currently_playing {
                format!("grid-template-columns: 40px minmax(0, 1fr) 200px 200px 64px 40px; column-gap: 1.5rem; background: color-mix(in oklab, var(--color-indigo-500) 12%, transparent); box-shadow: {selection_shadow};")
            } else {
                format!("grid-template-columns: 40px minmax(0, 1fr) 200px 200px 64px 40px; column-gap: 1.5rem; box-shadow: {selection_shadow};")
            },
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
            onmousedown: move |_| start_long_press(),
            onmouseup: move |_| cancel_long_press(),
            onmouseleave: move |_| cancel_long_press(),
            ontouchstart: move |_| start_long_press(),
            ontouchend: move |_| cancel_long_press(),
            oncontextmenu: move |evt| {
                evt.prevent_default();
                if !is_selection_mode {
                    on_click_menu.call(());
                }
            },

            div { class: "flex items-center w-10 shrink-0",
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
                    span { class: "text-xs text-slate-500", "{n}" }
                }
            }

            div { class: "flex items-center min-w-0 pr-4",
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

            div { class: "min-w-0 pr-4",
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

            div { class: "min-w-0 pr-4",
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
