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
use hooks::toast::toast;
use reader::models::Track;
use tracing::Instrument;

pub(crate) fn copy_to_clipboard(text: &str) {
    let value = serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string());
    let js = format!(
        "navigator.clipboard.writeText({value}).catch((e) => console.error('clipboard writeText failed', e));"
    );
    let _ = dioxus::document::eval(&js);
}

pub(crate) fn share_to_musicbrainz(release_id: Option<String>, artist: String, title: String) {
    spawn(
        async move {
            if let Some(url) =
                utils::musicbrainz::track_page_url(release_id.as_deref(), &artist, &title).await
            {
                copy_to_clipboard(&url);
                toast("Copied MusicBrainz link");
            } else {
                toast("Couldn't find this track on MusicBrainz");
            }
        }
        .instrument(tracing::info_span!("musicbrainz.fetch")),
    );
}

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
    on_view_metadata: Option<EventHandler<()>>,
    #[props(default = false)] is_selection_mode: bool,
    #[props(default = false)] is_selected: bool,
    #[props(default = false)] hide_delete: bool,
    on_select: Option<EventHandler<bool>>,
    on_long_press: Option<EventHandler<()>>,
    on_download: Option<EventHandler<()>>,
    on_start_radio: Option<EventHandler<()>>,
    #[props(default = false)] is_downloaded: bool,
    #[props(default = false)] is_downloading: bool,
    #[props(default = false)] is_currently_playing: bool,
    #[props(default = Vec::new())] selected_queue_tracks: Vec<Track>,
    #[props(default = None)] row_num: Option<usize>,
) -> Element {
    let config = use_context::<Signal<AppConfig>>();
    let active_source = use_context::<Signal<::server::source::ActiveSource>>();
    let mut ctrl = use_context::<PlayerController>();
    let nav_ctrl = use_context::<NavigationController>();
    let is_vaxry = config.read().ui_style == UiStyle::Vaxry;
    let show_row_images = config.read().show_row_images;
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
    let share_text = i18n::t("share_musicbrainz").to_string();
    let view_metadata_text = i18n::t("view_metadata").to_string();

    // The track to share, cloned once per layout closure (vaxry / normal) since
    // each moves it in; `share_track` picks the link form from the source.
    let share_track_vaxry = track.clone();
    let share_track_normal = track.clone();

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

    // `on_download` is only wired by sources that support downloads (servers),
    // so its presence is the gate — no separate source check needed.
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

    let delete_action_idx = if !hide_delete {
        let idx = actions.len();
        actions.push(MenuAction::new(delete_song_text.as_str(), "fa-solid fa-trash").destructive());
        Some(idx)
    } else {
        None
    };

    let share_idx = actions.len();
    actions.push(MenuAction::new(
        share_text.as_str(),
        "fa-solid fa-share-nodes",
    ));

    let mix_idx = if on_start_radio.is_some() {
        let idx = actions.len();
        actions.push(MenuAction::new(
            "Start radio",
            "fa-solid fa-tower-broadcast",
        ));
        Some(idx)
    } else {
        None
    };

    let has_view_metadata = on_view_metadata.is_some();
    let view_metadata_idx = if has_view_metadata {
        let idx = actions.len();
        actions.push(MenuAction::new(
            view_metadata_text.as_str(),
            "fa-solid fa-circle-info",
        ));
        Some(idx)
    } else {
        None
    };

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

    // File-type tag (MP3, FLAC, …) for local tracks. Server tracks have a
    // `TrackId::Server` id with no filesystem path, so they get no badge.
    let file_type = track
        .id
        .local_path()
        .and_then(|p| p.extension())
        .and_then(|e| e.to_str())
        .filter(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "mp3" | "flac" | "m4a" | "wav" | "ogg" | "opus" | "mp4" | "mka"
            )
        })
        .map(|e| e.to_uppercase());

    let columns_vaxry = if is_album {
        COLUMNS_VAXRY_ALBUM
    } else {
        COLUMNS_VAXRY
    };

    if is_vaxry {
        return rsx! {
            div {
                class: "track-row-draggable grid px-2 py-1.5 rounded-lg mx-1 group cursor-grab active:cursor-grabbing transition-colors hover:bg-white/5 select-none",
                style: if is_currently_playing {
                    format!("grid-template-columns: {columns_vaxry}; background: color-mix(in oklab, var(--color-indigo-500) 12%, transparent); box-shadow: {selection_shadow};")
                } else {
                    format!("grid-template-columns: {columns_vaxry}; box-shadow: {selection_shadow};")
                },
                onclick: move |evt| {
                    evt.stop_propagation();
                    if *long_press_occurred.read() {
                        long_press_occurred.set(false);
                        return;
                    }
                    if is_selection_mode {
                        handle_select_click(is_selected, is_selection_mode, on_select);
                    } else if cfg!(target_os = "android") {
                        // Mobile: a single tap plays (no double-click).
                        on_play.call(());
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
                    if !is_album && show_row_images {
                        div {
                            class: "w-8 h-8 rounded overflow-hidden shrink-0",
                            style: format!("background: url('{}') center/cover no-repeat, rgba(255,255,255,0.05);", utils::DEFAULT_COVER_SVG),
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
                            let has_album = !track.album.trim().is_empty();
                            move |evt: MouseEvent| {
                                evt.stop_propagation();
                                if !is_selection_mode {
                                    // Mobile always plays. Desktop drills into the
                                    // album — but only if the track actually has one.
                                    // Albumless tracks (uploads, music videos, YT
                                    // singles, Unknown Album from local) just play
                                    // on title click; otherwise we'd be navigating
                                    // into a meaningless "Singles" / "Unknown Album"
                                    // bucket.
                                    if cfg!(target_os = "android") || !has_album {
                                        on_play.call(());
                                    } else {
                                        nav_ctrl.navigate_to_album(album_id.clone());
                                    }
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
                    if let Some(ref ft) = file_type {
                        span {
                            class: "shrink-0 text-[9px] font-semibold uppercase px-1 py-0.5 rounded leading-none tracking-wide",
                            style: "background: rgba(255,255,255,0.08); color: var(--color-white); opacity: 0.5;",
                            "{ft}"
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
                            class: if track.album.trim().is_empty() {
                                "text-sm truncate"
                            } else {
                                "text-sm truncate cursor-pointer hover:underline"
                            },
                            style: "color: var(--color-white); opacity: 0.35;",
                            onclick: {
                                let album_id = track.album_id.clone();
                                let has_album = !track.album.trim().is_empty();
                                move |evt: MouseEvent| {
                                    evt.stop_propagation();
                                    if !is_selection_mode && has_album {
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
                            button_class: "active:scale-95".to_string(),
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
                                    && let Some(handler) = on_queue
                                {
                                    handler.call(());
                                    return;
                                }
                                if idx == add_to_playlist_idx {
                                    on_add_to_playlist.call(());
                                } else if remove_action_idx == Some(idx) {
                                    if let Some(handler) = on_remove_from_playlist { handler.call(()); }
                                } else if has_download && idx == download_action_idx {
                                    if let Some(handler) = on_download { handler.call(()); }
                                } else if idx == share_idx {
                                    let src = active_source.peek().clone();
                                    share_track(share_track_vaxry.clone(), src);
                                    on_close_menu.call(());
                                } else if mix_idx == Some(idx) {
                                    if let Some(handler) = on_start_radio {
                                        handler.call(());
                                    }
                                    on_close_menu.call(());
                                } else if view_metadata_idx == Some(idx) {
                                    if let Some(handler) = on_view_metadata { handler.call(()); }
                                    on_close_menu.call(());
                                } else if Some(idx) == delete_action_idx {
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
    let column_gap = if cfg!(target_os = "android") {
        "0.5rem"
    } else {
        "1.5rem"
    };

    // normal UI
    return rsx! {
        div {
            class: "track-row-draggable grid items-center h-14 p-2 rounded-lg hover:bg-white/5 group transition-colors relative select-none cursor-grab active:cursor-grabbing",
            style: if is_currently_playing {
                format!("grid-template-columns: {columns_normal}; column-gap: {column_gap}; background: color-mix(in oklab, var(--color-indigo-500) 12%, transparent); box-shadow: {selection_shadow};")
            } else {
                format!("grid-template-columns: {columns_normal}; column-gap: {column_gap}; box-shadow: {selection_shadow};")
            },
            draggable: "false",
            onclick: move |evt| {
                evt.stop_propagation();
                if *long_press_occurred.read() {
                    long_press_occurred.set(false);
                    return;
                }
                if !is_selection_mode && cfg!(target_os = "android") {
                    // Mobile: a single tap plays (no double-click).
                    on_play.call(());
                } else {
                    handle_select_click(is_selected, is_selection_mode, on_select);
                }
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
                if !is_album && show_row_images {
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
                                // Mobile: tapping the title plays the track instead of
                                // navigating to the album.
                                if cfg!(target_os = "android") {
                                    on_play.call(());
                                } else {
                                    nav_ctrl.navigate_to_album(album_id.clone());
                                }
                            }
                        }
                    },
                    ondoubleclick: move |evt| evt.stop_propagation(),
                    "{track.title}"
                }
                if let Some(ref ft) = file_type {
                    span {
                        class: "shrink-0 ml-2 text-[9px] font-semibold uppercase px-1 py-0.5 rounded leading-none tracking-wide",
                        style: "background: rgba(255,255,255,0.08); color: var(--color-white); opacity: 0.5;",
                        "{ft}"
                    }
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
                        button_class: "opacity-0 group-hover:opacity-100 focus:opacity-100 active:scale-95".to_string(),
                        anchor: "right".to_string(),
                        on_action: move |idx: usize| {
                            if let Some(play_next_idx) = play_next_idx
                                && idx == play_next_idx
                            {
                                ctrl.queue_play_next(vec![play_next_track_normal.clone()]);
                                on_close_menu.call(());
                                return;
                            }
                            if let Some(queue_idx) = add_to_queue_idx
                                && idx == queue_idx
                            {
                                if let Some(handler) = on_queue {
                                    handler.call(());
                                }
                                return;
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
                            } else if idx == share_idx {
                                let src = active_source.peek().clone();
                                share_track(share_track_normal.clone(), src);
                                on_close_menu.call(());
                            } else if mix_idx == Some(idx) {
                                if let Some(handler) = on_start_radio {
                                    handler.call(());
                                }
                                on_close_menu.call(());
                            } else if view_metadata_idx == Some(idx) {
                                if let Some(handler) = on_view_metadata {
                                    handler.call(());
                                }
                                on_close_menu.call(());
                            } else if Some(idx) == delete_action_idx {
                                on_delete.call(());
                            }
                        },
                    }
                }
            }
        }
    };
}

/// The `on_start_radio` handler for a track row: `Some` iff the active source
/// supports radio ([`Capabilities::radio`]), else `None` (so the row hides the
/// "Start radio" action). Lets every call site wire radio in one line without
/// repeating the capability gate or context plumbing.
///
/// Reads context via `consume_context`, never a `use_*` hook: call sites invoke
/// this once per visible row, so a hook here would register a per-row-count
/// number of hooks and panic the parent on rules-of-hooks when the row count
/// changes (e.g. an empty server library filling in after a sync).
pub fn radio_handler(track: Track) -> Option<EventHandler<()>> {
    let ctrl = consume_context::<PlayerController>();
    let active_source = consume_context::<Signal<::server::source::ActiveSource>>();
    let can_radio = active_source.read().capabilities().radio;
    can_radio.then(|| {
        EventHandler::new(move |_| {
            let src = active_source.peek().clone();
            play_radio(track.clone(), src, ctrl)
        })
    })
}

/// Start radio seeded from a track and play the generated queue. The radio
/// operation lives in the source layer ([`MediaSource::start_radio`]); this just
/// resolves the track's source, awaits it, and hands the result to the player.
/// Call sites wire this into `on_start_radio` only when `capabilities().radio`.
pub fn play_radio(
    track: Track,
    source: ::server::source::ActiveSource,
    mut ctrl: PlayerController,
) {
    let seed = track.id.key().into_owned();
    spawn(
        async move {
            match source.start_radio(&seed).await {
                Ok(tracks) if !tracks.is_empty() => ctrl.play_queue_linear(tracks),
                Ok(_) => tracing::debug!(seed = %seed, "radio returned empty queue"),
                Err(e) => tracing::warn!(seed = %seed, error = %e, "radio failed"),
            }
        }
        .instrument(tracing::info_span!("radio.start")),
    );
}

/// Copy a shareable link for a track: its source's public web URL when it has
/// one (YT Music or Spotify), else fall back to a MusicBrainz lookup by metadata. The provider
/// URL knowledge lives in the source impl ([`MediaSource::web_url`]), not here.
pub fn share_track(track: Track, source: ::server::source::ActiveSource) {
    if let Some(url) = source.web_url(&track) {
        copy_to_clipboard(&url);
        toast("Copied link");
    } else {
        share_to_musicbrainz(track.musicbrainz_release_id, track.artist, track.title);
    }
}
