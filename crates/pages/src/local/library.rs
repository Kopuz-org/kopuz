use components::header::Header;
use components::metadata_modal::MetadataModal;
use components::playlist_modal::PlaylistModal;
use components::selection_bar::SelectionBar;
use components::stat_card::StatCard;
use components::track_row::TrackRow;
use components::virtual_scroll::{VirtualScrollView, use_virtual_scroll};
use config::{AppConfig, UiStyle};
use dioxus::prelude::*;
use hooks::use_library_items::use_library_items;
use hooks::use_player_controller::PlayerController;
use kopuz_route::Route;
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
    let track_sort = items.track_sort;
    let mut scroll_positions = use_context::<Signal<std::collections::HashMap<Route, f64>>>();
    let saved_scroll = scroll_positions
        .peek()
        .get(&Route::Library)
        .copied()
        .unwrap_or(0.0);
    let scroll_stat = use_signal(move || saved_scroll);
    let container_height = use_signal(|| 0.0_f64);
    use_effect(move || {
        let curr = track_sort.read().clone();
        if config.peek().track_sort != curr {
            config.write().track_sort = curr;
        }
    });
    let mut ctrl = use_context::<PlayerController>();
    let mut active_menu_track = use_signal(|| None::<PathBuf>);
    let mut show_playlist_modal = use_signal(|| false);
    let mut selected_track_for_playlist = use_signal(|| None::<PathBuf>);
    let mut metadata_track = use_signal(|| None::<reader::models::Track>);
    let mut is_selection_mode = use_signal(|| false);
    let mut selected_tracks = use_signal(|| HashSet::<PathBuf>::new());
    let displayed_tracks = use_memo(move || (items.all_tracks)());
    // Sort fields to offer, derived from the data actually loaded.
    let available_sort_fields = use_memo(move || {
        let lib = library.read();
        let album_years = reader::sort::album_year_map(&lib.albums);
        reader::sort::available_track_fields(&lib.tracks, &album_years)
    });
    let album_covers = use_memo(move || (items.album_covers)());
    let cover_urls_memo = use_memo(move || std::sync::Arc::new(album_covers()));
    let cover_urls = cover_urls_memo();
    let (total_tracks, is_empty) = {
        let t = displayed_tracks.read();
        (t.len(), t.is_empty())
    };
    let scroll_info = use_virtual_scroll(
        *scroll_stat.read(),
        *container_height.read(),
        total_tracks,
        ITEM_HEIGHT,
    );
    let all_selected = !is_empty && {
        let tracks = displayed_tracks.read();
        let sel = selected_tracks.read();
        sel.len() >= tracks.len() && tracks.iter().all(|track| sel.contains(&track.path))
    };
    let currently_playing_idx: Option<usize> = use_memo(move || {
        let queue = ctrl.queue.read();
        let current_index = *ctrl.current_queue_index.read();
        if let Some(q_idx) = ctrl.get_queue_index(current_index) {
            let all = displayed_tracks.read();
            if queue.len() == all.len()
                && queue.iter().zip(all.iter()).all(|(q, t)| q.path == t.path)
            {
                Some(q_idx)
            } else {
                None
            }
        } else {
            None
        }
    })();
    let tracks_nodes = {
        let all_tracks = displayed_tracks.read();
        all_tracks
            .iter()
            .enumerate()
            .skip(scroll_info.start_index)
            .take(scroll_info.items_to_render)
            .map(|(idx, track)| {
                let track = track.clone();
                let track_menu = track.clone();
                let track_add = track.clone();
                let track_queue = track.clone();
                let track_meta = track.clone();
                let track_delete = track.clone();
                let track_path = track.path.clone();
                let is_currently_playing = currently_playing_idx == Some(idx);
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
                            is_album: false,
                            is_currently_playing,
                            is_selection_mode: is_selection_mode(),
                            is_selected,
                            row_num: Some(idx + 1),
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
                            on_view_metadata: move |_| {
                                metadata_track.set(Some(track_meta.clone()));
                                active_menu_track.set(None);
                            },
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
    let is_modern = config.read().ui_style == UiStyle::Modern;
    rsx! {
        div {
            class: if cfg!(target_os = "android") { "px-3 pt-3 absolute inset-0 flex flex-col overflow-x-hidden" } else if is_modern { "px-6 pt-6 absolute inset-0 flex flex-col" } else { "px-8 pt-8 absolute inset-0 flex flex-col" },
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
            if let Some(track) = metadata_track.read().clone() {
                MetadataModal {
                    track: track.clone(),
                    on_close: move |_| metadata_track.set(None),
                    on_save: move |edits: reader::models::TrackEdits| {
                        let path = track.path.clone();
                        match reader::write_tags(&path, &edits) {
                            Ok(()) => {
                                let mut lib = library.write();
                                if let Some(t) = lib.tracks.iter_mut().find(|t| t.path == path) {
                                    t.title = edits.title.trim().to_string();
                                    t.artist = edits.artist.trim().to_string();
                                    t.artists = edits
                                        .artist
                                        .split([';', ','])
                                        .map(|a| a.trim().to_string())
                                        .filter(|s| !s.is_empty())
                                        .collect();
                                    t.album = edits.album.trim().to_string();
                                    t.track_number = edits.track_number;
                                    t.disc_number = edits.disc_number;
                                    t.album_id = reader::metadata::make_album_id(
                                        edits.album.trim(),
                                        edits.artist.trim(),
                                    );
                                }
                                drop(lib);
                                metadata_track.set(None);
                            }
                            Err(e) => {
                                tracing::error!("failed to write tags for {}: {}", path.display(), e);
                            }
                        }
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
                        let tracks: Vec<_> = displayed_tracks
                            .read()
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
                    },
                }
            }
            div {
                class: "flex items-center justify-between mb-6",
                if is_modern {
                    div {
                        p {
                            class: "text-[10px] font-bold tracking-widest uppercase mb-0.5 text-white/35",
                            "{i18n::t(\"library\")}"
                        }
                        h1 { class: "text-2xl font-bold text-white", "{i18n::t(\"your_library\")}" }
                    }
                } else {
                    h1 { class: "text-3xl font-bold text-white", "{i18n::t(\"your_library\")}" }
                }
                button {
                    class: "text-white/60 hover:text-white transition-colors p-2 rounded-full hover:bg-white/10",
                    title: i18n::t("rescan_library").to_string(),
                    onclick: move |_| on_rescan.call(()),
                    i { class: "fa-solid fa-rotate" }
                }
            }
            div {
                class: if cfg!(target_os = "android") { "grid grid-cols-4 gap-2 mb-4" } else { "grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4 mb-12" },
                {
                    let lib = library.read();
                    let album_count = lib
                        .albums
                        .iter()
                        .map(|a| a.title.to_lowercase())
                        .collect::<std::collections::HashSet<_>>()
                        .len();
                    rsx! {
                        StatCard { label: i18n::t("tracks").to_string(), value: "{lib.tracks.len()}", icon: "fa-music" }
                        StatCard { label: i18n::t("albums").to_string(), value: "{album_count}", icon: "fa-compact-disc" }
                        StatCard { label: i18n::t("artists").to_string(), value: "{(items.artist_count)()}", icon: "fa-user" }
                        StatCard {
                            label: i18n::t("playlists").to_string(),
                            value: "{playlist_store.read().playlists.len()}",
                            icon: "fa-list",
                        }
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
                                selected_tracks
                                    .set(tracks.into_iter().map(|track| track.path).collect());
                                is_selection_mode.set(true);
                            }
                        },
                        if all_selected {
                            i { class: "fa-solid fa-check", style: "font-size: 9px;" }
                        }
                    }
                    h2 { class: "text-xl font-semibold text-white/80", "{i18n::t(\"tracks\")}" }
                }
                components::sort_control::SortControl::<config::TrackSortField> {
                    criteria: track_sort,
                    available: available_sort_fields(),
                }
            }
            Header { is_modern: is_modern, is_album: false }
            VirtualScrollView {
                id: "library-scroll".to_string(),
                class: if cfg!(target_os = "android") { "flex-1 overflow-y-auto overflow-x-hidden pb-20".to_string() } else { "flex-1 overflow-y-auto pb-20".to_string() },
                scroll_stat,
                container_height,
                item_height: ITEM_HEIGHT,
                saved_scroll,
                top_pad: scroll_info.top_pad,
                bottom_pad: scroll_info.bottom_pad,
                onscroll: move |scroll| {
                    scroll_positions.write().insert(Route::Library, scroll);
                },
                if is_empty {
                    p { class: "text-slate-500 italic", "{i18n::t(\"no_tracks_found\")}" }
                } else {
                    {tracks_nodes.into_iter()}
                }
            }
        }
    }
}
