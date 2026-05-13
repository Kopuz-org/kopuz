use components::playlist_modal::PlaylistModal;
use components::selection_bar::SelectionBar;
use components::stat_card::StatCard;
use components::track_row::TrackRow;
use config::AppConfig;
use dioxus::prelude::*;
use hooks::use_library_items::use_library_items;
use hooks::use_player_controller::PlayerController;
use reader::Library;
use std::collections::HashSet;
use std::path::PathBuf;

const ITEM_HEIGHT: f64 = 60.0; // 60px: p-2 padding (16px*2=32) + content height (~28px)

#[component]
pub fn LocalLibrary(
    library: Signal<Library>,
    config: Signal<AppConfig>,
    playlist_store: Signal<reader::PlaylistStore>,
    on_rescan: EventHandler,
    mut queue: Signal<Vec<reader::models::Track>>,
) -> Element {
    let items = use_library_items(library);
    let mut sort_order = items.sort_order;
    let mut scroll_stat = use_signal(|| 0.0);
    let mut container_height = use_signal(|| f64::NAN); // Set on mount

    use_effect(move || {
        let curr = sort_order.read().clone();
        if config.peek().sort_order != curr {
            config.write().sort_order = curr;
        }
    });

    let mut ctrl = use_context::<PlayerController>();
    let mut active_menu_track = use_signal(|| None::<PathBuf>);
    let mut show_playlist_modal = use_signal(|| false);
    let mut selected_track_for_playlist = use_signal(|| None::<PathBuf>);

    // Multi-selection state
    let mut is_selection_mode = use_signal(|| false);
    let mut selected_tracks = use_signal(|| HashSet::<PathBuf>::new());

    let displayed_tracks = use_memo(move || (items.all_tracks)());
    let album_covers = use_memo(move || (items.album_covers)());

    let cover_urls = std::sync::Arc::new(album_covers());

    let scroll_top = *scroll_stat.read();
    let row_height = ITEM_HEIGHT;
    let container_h = *container_height.read();
    let window_size = if container_h.is_nan() {
        0
    } else {
        (container_h / row_height).ceil() as usize
    };
    let buffer_size = 10;

    let (total_tracks, is_empty) = {
        let t = displayed_tracks.read();
        (t.len(), t.is_empty())
    };

    let all_selected = !is_empty
        && displayed_tracks()
            .iter()
            .all(|track| selected_tracks.read().contains(&track.path));

    let start_index = {
        let max_start = total_tracks.saturating_sub(1);
        let calc = (scroll_top - (buffer_size as f64) * row_height) / row_height;
        (calc.floor().max(0.0) as usize).min(max_start)
    };

    let end_index = {
        let last_index = start_index + 2 * buffer_size + window_size;
        let last_index_inclusive = last_index.saturating_sub(1);
        if total_tracks == 0 {
            0
        } else {
            last_index_inclusive.min(total_tracks - 1)
        }
    };

    let items_to_render = if total_tracks == 0 {
        0
    } else {
        (end_index + 1).saturating_sub(start_index)
    };

    let top_pad = (start_index as f64) * row_height;

    let bottom_pad = {
        let total_height = (total_tracks as f64) * row_height;
        let rendered_height = (items_to_render as f64) * row_height;
        (total_height - rendered_height - top_pad).max(0.0)
    };

    let tracks_nodes = {
        let all_tracks = displayed_tracks.read();
        all_tracks
            .iter()
            .enumerate()
            .skip(start_index)
        .take(items_to_render)
        .map(|(idx, track)| {
            let track = track.clone();
            let track_menu = track.clone();
            let track_add = track.clone();
            let track_queue = track.clone();
            let track_delete = track.clone();
            let track_path = track.path.clone();
            let track_select = track.path.clone();
            let cover_urls = std::sync::Arc::clone(&cover_urls);
            let track_key = track.path.display().to_string();
            let is_menu_open = active_menu_track.read().as_ref() == Some(&track.path);
            let is_selected = selected_tracks.read().contains(&track_path);
            let cover_url = cover_urls.get(&track.album_id).cloned().flatten();

            rsx! {
div {
    key: "{track_key}",
    style: "height: {ITEM_HEIGHT}px;",
                    TrackRow {
                        track: track.clone(),
                        cover_url: cover_url.clone(),
                        is_menu_open,
                        is_selection_mode: is_selection_mode(),
                        is_selected,
                        on_long_press: move |_| {
                            is_selection_mode.set(true);
                            selected_tracks.write().insert(track_path.clone());
                        },
                        on_select: move |selected| {
                            if selected {
                                is_selection_mode.set(true);
                                selected_tracks.write().insert(track_select.clone());
                            } else {
                                selected_tracks.write().remove(&track_select);
                                if selected_tracks.read().is_empty() {
                                    is_selection_mode.set(false);
                                }
                            }
                        },
                        on_click_menu: move |_| {
                            if active_menu_track.read().as_ref() == Some(&track_menu.path) {
                                active_menu_track.set(None);
                            } else {
                                active_menu_track.set(Some(track_menu.path.clone()));
                            }
                        },
                        on_add_to_playlist: move |_| {
                            selected_track_for_playlist.set(Some(track_add.path.clone()));
                            show_playlist_modal.set(true);
                            active_menu_track.set(None);
                        },
                        on_queue: move |_| {
                            ctrl.add_to_queue(vec![track_queue.clone()]);
                            active_menu_track.set(None);
                        },
                        on_close_menu: move |_| active_menu_track.set(None),
                        on_delete: move |_| {
                            active_menu_track.set(None);
                            if std::fs::remove_file(&track_delete.path).is_ok() {
                                library.write().remove_track(&track_delete.path);
                            }
                        },
                        on_play: move |_| {
                            queue.set(displayed_tracks());
                            ctrl.play_track(idx);
                        },
                    }
                }
            }
        })
        .collect::<Vec<_>>()
    };

    rsx! {
             div {
                 class: "p-8 relative min-h-full flex flex-col",

            if *show_playlist_modal.read() {
                PlaylistModal {
                    playlist_store,
                    is_jellyfin: false,
                    on_close: move |_| {
                        show_playlist_modal.set(false);
                        if is_selection_mode() {
                            is_selection_mode.set(false);
                            selected_tracks.write().clear();
                        }
                    },
                    on_add_to_playlist: move |playlist_id: String| {
                        let mut store = playlist_store.write();
                        if let Some(playlist) = store.playlists.iter_mut().find(|p| p.id == playlist_id) {
                            if is_selection_mode() {
                                for path in selected_tracks.read().iter() {
                                    if !playlist.tracks.contains(path) {
                                        playlist.tracks.push(path.clone());
                                    }
                                }
                            } else if let Some(path) = selected_track_for_playlist.read().clone() {
                                if !playlist.tracks.contains(&path) {
                                    playlist.tracks.push(path);
                                }
                            }
                        }
                        show_playlist_modal.set(false);
                        active_menu_track.set(None);
                        is_selection_mode.set(false);
                        selected_tracks.write().clear();
                    },
                    on_create_playlist: move |name: String| {
                        let mut tracks = Vec::new();
                        if is_selection_mode() {
                            tracks = selected_tracks.read().iter().cloned().collect();
                        } else if let Some(path) = selected_track_for_playlist.read().clone() {
                            tracks.push(path);
                        }

                        let mut store = playlist_store.write();
                        store.playlists.push(reader::models::Playlist {
                            id: uuid::Uuid::new_v4().to_string(),
                            name,
                            tracks,
                            cover_path: None,
                        });
                        show_playlist_modal.set(false);
                        active_menu_track.set(None);
                        is_selection_mode.set(false);
                        selected_tracks.write().clear();
                    },
                }
            }

            if is_selection_mode() {
                SelectionBar {
                    count: selected_tracks.read().len(),
                    on_add_to_queue: move |_| {
                        let selected = selected_tracks.read().clone();
                        if selected.is_empty() {
                            return;
                        }
                        let tracks: Vec<_> = displayed_tracks.read()
                            .iter()
                            .filter(|t| selected.contains(&t.path))
                            .cloned()
                            .collect();
                        if !tracks.is_empty() {
                            ctrl.add_to_queue(tracks);
                        }
                        selected_tracks.write().clear();
                        is_selection_mode.set(false);
                    },
                    on_add_to_playlist: move |_| {
                        show_playlist_modal.set(true);
                    },
                    on_delete: move |_| {
                        let paths: Vec<_> = selected_tracks.read().iter().cloned().collect();
                        for path in paths {
                            if std::fs::remove_file(&path).is_ok() {
                                library.write().remove_track(&path);
                            }
                        }
                        selected_tracks.write().clear();
                        is_selection_mode.set(false);
                    },
                    on_cancel: move |_| {
                        is_selection_mode.set(false);
                        selected_tracks.write().clear();
                    }
                }
            }

            div {
                class: "flex items-center justify-between mb-6",
                h1 { class: "text-3xl font-bold text-white", "{i18n::t(\"your_library\")}" }
                button {
                    class: "text-white/60 hover:text-white transition-colors p-2 rounded-full hover:bg-white/10",
                    title: i18n::t("rescan_library").to_string(),
                    onclick: move |_| on_rescan.call(()),
                    i { class: "fa-solid fa-rotate" }
                }
            }

            div {
                class: "grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4 mb-12",
                {
                    let lib = library.read();
                    let album_count = lib.albums.iter()
                        .map(|a| a.title.to_lowercase())
                        .collect::<std::collections::HashSet<_>>()
                        .len();
                    rsx! {
                        StatCard { label: i18n::t("tracks").to_string(),    value: "{lib.tracks.len()}",  icon: "fa-music" }
                        StatCard { label: i18n::t("albums").to_string(),    value: "{album_count}",  icon: "fa-compact-disc" }
                        StatCard { label: i18n::t("artists").to_string(),   value: "{(items.artist_count)()}", icon: "fa-user" }
                        StatCard { label: i18n::t("playlists").to_string(), value: "{playlist_store.read().playlists.len()}", icon: "fa-list" }
                    }
                }
            }

            div {
                class: "flex items-center justify-between mb-4",
                div { class: "flex items-center gap-3",
                    button {
                        class: if all_selected {
                            "w-4 h-4 rounded border border-indigo-400 bg-indigo-500 text-white flex items-center justify-center transition-colors"
                        } else {
                            "w-4 h-4 rounded border border-white/20 bg-white/5 hover:border-white/50 transition-colors"
                        },
                        aria_label: "Select all tracks",
                        disabled: is_empty,
                        onclick: move |_| {
                            let tracks = displayed_tracks();
                            if all_selected {
                                selected_tracks.write().clear();
                                is_selection_mode.set(false);
                            } else {
                                selected_tracks.set(tracks.into_iter().map(|track| track.path).collect());
                                is_selection_mode.set(true);
                            }
                        },
                        if all_selected {
                            i { class: "fa-solid fa-check", style: "font-size: 9px;" }
                        }
                    }
                    h2 { class: "text-xl font-semibold text-white/80", "{i18n::t(\"tracks\")}" }
                }
                div {
                    class: "flex space-x-1 bg-white/5 border border-white/5 p-1 rounded-lg",
                    button {
                        class: if *sort_order.read() == config::SortOrder::Title {
                            "px-3 py-1 text-xs rounded-md bg-white/10 text-white font-medium transition-all"
                        } else {
                            "px-3 py-1 text-xs rounded-md text-white/40 hover:text-white/80 transition-all"
                        },
                        onclick: move |_| sort_order.set(config::SortOrder::Title),
                        "{i18n::t(\"title\")}"
                    }
                    button {
                        class: if *sort_order.read() == config::SortOrder::Artist {
                            "px-3 py-1 text-xs rounded-md bg-white/10 text-white font-medium transition-all"
                        } else {
                            "px-3 py-1 text-xs rounded-md text-white/40 hover:text-white/80 transition-all"
                        },
                        onclick: move |_| sort_order.set(config::SortOrder::Artist),
                        "{i18n::t(\"artist\")}"
                    }
                    button {
                        class: if *sort_order.read() == config::SortOrder::Album {
                            "px-3 py-1 text-xs rounded-md bg-white/10 text-white font-medium transition-all"
                        } else {
                            "px-3 py-1 text-xs rounded-md text-white/40 hover:text-white/80 transition-all"
                        },
                        onclick: move |_| sort_order.set(config::SortOrder::Album),
                        "{i18n::t(\"album\")}"
                    }
                }
            }

            div {
                class: "flex-1 overflow-y-auto pb-20",
                onmounted: move |event| {
                    spawn(async move {
                        if let Ok(window) = event.get_client_rect().await {
                            container_height.set(window.height());
                        }
                    });
                },
                onscroll: move |event| {
                    let new_scroll = event.scroll_top();
                    let old_row = (*scroll_stat.peek() / ITEM_HEIGHT).floor() as i64;
                    let new_row = (new_scroll / ITEM_HEIGHT).floor() as i64;
                    if new_row != old_row {
                        scroll_stat.set(new_scroll);
                    }
                    let height = event.client_height() as f64;
                    if (height - *container_height.peek()).abs() > 1.0 {
                        container_height.set(height);
                    }
                },
                if is_empty {
                    p { class: "text-slate-500 italic", "{i18n::t(\"no_tracks_found\")}" }
                } else {
                    div { style: "height: {top_pad}px; flex-shrink: 0;" }
                    {tracks_nodes.into_iter()}
                    div { style: "height: {bottom_pad}px; flex-shrink: 0;" }
                }
            }
        }
    }
}
