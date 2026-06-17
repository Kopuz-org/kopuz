//! Source-agnostic Library page (issue #35). One component for local and any
//! server: a windowed track list with stat cards and multi-select. The refresh
//! action (filesystem rescan vs remote sync), per-row affordances (tag edit,
//! delete-from-disk, download) and the selection bar all gate on the resolved
//! source's [`Capabilities`](server::source::Capabilities) — no `is_server()`.

use components::header::Header;
use components::metadata_modal::MetadataModal;
use components::playlist_modal::PlaylistModal;
use components::selection_bar::SelectionBar;
use components::stat_card::StatCard;
use components::track_row::TrackRow;
use components::virtual_scroll::{VirtualScrollView, use_virtual_scroll};
use config::{AppConfig, UiStyle};
use db::{Page, TrackFilter, TrackSort};
use dioxus::prelude::*;
use hooks::db_reactivity::Table;
use hooks::use_db_queries::{
    use_active_source, use_albums, use_artists, use_playlists, use_tracks_window,
};
use hooks::use_player_controller::PlayerController;
use kopuz_route::Route;
use std::collections::HashSet;
use std::path::PathBuf;

use crate::server::download_manager::{DownloadQueue, DownloadStatus, queue_downloads};

const ITEM_HEIGHT: f64 = 60.0; // 60px: p-2 padding (16px*2=32) + content height (~28px)

#[component]
pub fn LibraryPage(
    mut config: Signal<AppConfig>,
    on_rescan: EventHandler,
    player: Signal<player::player::Player>,
    mut is_playing: Signal<bool>,
    mut current_playing: Signal<u64>,
    mut current_song_cover_url: Signal<String>,
    mut current_song_title: Signal<String>,
    mut current_song_artist: Signal<String>,
    mut current_song_duration: Signal<u64>,
    mut current_song_progress: Signal<u64>,
    mut queue: Signal<Vec<reader::models::Track>>,
    mut current_queue_index: Signal<usize>,
) -> Element {
    let gens = hooks::db_reactivity::use_generations();
    let source = use_active_source();
    let active_source = use_context::<Signal<::server::source::ActiveSource>>();
    let caps = use_memo(move || active_source.read().capabilities());
    let download_queue = use_context::<Signal<DownloadQueue>>();

    let initial_sort_order = config.read().sort_order.clone();
    let mut sort_order = use_signal(move || initial_sort_order);
    let filter = use_memo(move || TrackFilter {
        source: source(),
        sort: match *sort_order.read() {
            config::SortOrder::Title => TrackSort::Title,
            config::SortOrder::Artist => TrackSort::Artist,
            config::SortOrder::Album => TrackSort::Album,
        },
        ..Default::default()
    });
    use_effect(move || {
        let curr = sort_order.read().clone();
        if config.peek().sort_order != curr {
            config.write().sort_order = curr;
        }
    });

    let albums_res = use_albums(source);
    let artists_res = use_artists(source);
    let playlists_res = use_playlists();

    let mut scroll_positions = use_context::<Signal<std::collections::HashMap<Route, f64>>>();
    let saved_scroll = scroll_positions
        .peek()
        .get(&Route::Library)
        .copied()
        .unwrap_or(0.0);
    let scroll_stat = use_signal(move || saved_scroll);
    let container_height = use_signal(|| 0.0_f64);
    let mut total_rows = use_signal(|| 0_usize);
    let page = use_memo(move || {
        let info = use_virtual_scroll(
            *scroll_stat.read(),
            *container_height.read(),
            total_rows(),
            ITEM_HEIGHT,
        );
        Page {
            offset: info.start_index as u32,
            limit: info.items_to_render as u32,
        }
    });
    let window = use_tracks_window(filter, page);
    use_effect(move || {
        let total = window.total.read().unwrap_or(0) as usize;
        if *total_rows.peek() != total {
            total_rows.set(total);
        }
    });

    // Remote sync (servers). Local never calls this — its refresh is `on_rescan`.
    let mut is_loading = use_signal(|| false);
    let mut has_fetched = use_signal(|| false);
    let mut fetch_generation = use_signal(|| 0usize);
    let mut sync_server = move || {
        has_fetched.set(true);
        is_loading.set(true);
        fetch_generation.with_mut(|g| *g += 1);
        let current_gen = *fetch_generation.peek();
        spawn(async move {
            if *fetch_generation.read() == current_gen {
                let _ = crate::server::subsonic_sync::sync_server_library(true).await;
                if *fetch_generation.read() == current_gen {
                    is_loading.set(false);
                }
            }
        });
    };
    // First visit with an empty server library → auto-pull once.
    use_effect(move || {
        if !caps().sync {
            return;
        }
        if !*has_fetched.read()
            && let Some(total) = *window.total.read()
        {
            if total == 0 {
                sync_server();
            } else {
                has_fetched.set(true);
            }
        }
    });

    let mut ctrl = use_context::<PlayerController>();
    let mut active_menu_track = use_signal(|| None::<reader::TrackId>);
    let mut show_playlist_modal = use_signal(|| false);
    let mut selected_track_for_playlist = use_signal(|| None::<reader::TrackId>);
    let mut metadata_track = use_signal(|| None::<reader::models::Track>);
    let mut is_selection_mode = use_signal(|| false);
    let mut selected_tracks = use_signal(HashSet::<reader::TrackId>::new);

    // album_id → cover-path, for the per-row cover seam (local uses it, a server
    // ignores it and resolves from the track's own ref).
    let album_cover_paths = use_memo(move || {
        albums_res
            .read()
            .clone()
            .unwrap_or_default()
            .iter()
            .map(|a| (a.id.clone(), a.cover_path.clone()))
            .collect::<std::collections::HashMap<String, Option<PathBuf>>>()
    });

    let total_tracks = total_rows();
    let is_empty = total_tracks == 0;
    let scroll_info = use_virtual_scroll(
        *scroll_stat.read(),
        *container_height.read(),
        total_tracks,
        ITEM_HEIGHT,
    );
    let all_selected = !is_empty && selected_tracks.read().len() >= total_tracks;
    let currently_playing_idx: Option<usize> = {
        let current_index = *ctrl.current_queue_index.read();
        ctrl.get_queue_index(current_index)
            .filter(|_| ctrl.queue.read().len() == total_tracks)
    };

    let tracks_nodes = {
        let cap = caps();
        let conf = config.read();
        let covers = album_cover_paths.read();
        let window_rows = window.rows.read().clone().unwrap_or_default();
        let row_offset = window_rows.offset as usize;
        window_rows
            .rows
            .into_iter()
            .enumerate()
            .map(|(i, track)| {
                let idx = row_offset + i;
                let track_menu = track.clone();
                let track_add = track.clone();
                let track_queue = track.clone();
                let track_meta = track.clone();
                let track_delete = track.clone();
                let track_radio = track.clone();
                let track_path = track.id.clone();
                let track_select = track.id.clone();
                let track_key = track.id.uid();
                let is_currently_playing = currently_playing_idx == Some(idx)
                    && ctrl
                        .queue
                        .read()
                        .get(idx)
                        .map(|q| q.id == track.id)
                        .unwrap_or(false);
                let is_menu_open = active_menu_track.read().as_ref() == Some(&track.id);
                let is_selected = selected_tracks.read().contains(&track_path);
                let cover_url = ::server::cover::track(
                    &conf,
                    &track,
                    covers.get(&track.album_id).and_then(|p| p.as_deref()),
                    80,
                );

                // Download state (servers only).
                let item_id: String = track.id.key().to_string();
                let is_downloaded = cap.downloads
                    && conf
                        .offline_tracks
                        .get(&item_id)
                        .map(|p| std::path::Path::new(p).exists())
                        .unwrap_or(false);
                let is_downloading = cap.downloads
                    && download_queue.read().items.iter().any(|i| {
                        i.id == item_id
                            && matches!(
                                i.status,
                                DownloadStatus::Queued | DownloadStatus::Downloading
                            )
                    });
                let item_id_dl = item_id.clone();
                let track_title = track.title.clone();
                let track_artist = track.artist.clone();

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
                            is_downloaded,
                            is_downloading,
                            hide_delete: !cap.delete_from_disk,
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
                                if active_menu_track.read().as_ref() == Some(&track_menu.id) {
                                    active_menu_track.set(None);
                                } else {
                                    active_menu_track.set(Some(track_menu.id.clone()));
                                }
                            },
                            on_add_to_playlist: move |_| {
                                selected_track_for_playlist.set(Some(track_add.id.clone()));
                                show_playlist_modal.set(true);
                                active_menu_track.set(None);
                            },
                            on_queue: move |_| {
                                ctrl.add_to_queue(vec![track_queue.clone()]);
                                active_menu_track.set(None);
                            },
                            on_close_menu: move |_| active_menu_track.set(None),
                            on_view_metadata: caps().edit_tags.then(|| EventHandler::new(move |_| {
                                metadata_track.set(Some(track_meta.clone()));
                                active_menu_track.set(None);
                            })),
                            on_delete: move |_| {
                                active_menu_track.set(None);
                                if caps().delete_from_disk
                                    && let Some(p) = track_delete.id.local_path()
                                    && std::fs::remove_file(p).is_ok()
                                {
                                    let s = consume_context::<::server::source::LocalHandle>().0.clone();
                                    let key = track_delete.id.key().into_owned();
                                    spawn(async move {
                                        if s.delete_tracks(&[key]).await.is_ok() {
                                            gens.bump(Table::Tracks);
                                        }
                                    });
                                }
                            },
                            on_download: caps().downloads.then(|| EventHandler::new(move |_| {
                                if !is_downloaded {
                                    active_menu_track.set(None);
                                    queue_downloads(
                                        vec![(item_id_dl.clone(), track_title.clone(), track_artist.clone())],
                                        config,
                                        download_queue,
                                    );
                                }
                            })),
                            on_start_radio: components::track_row::radio_handler(track_radio.clone()),
                            on_play: move |_| {
                                let read_db = consume_context::<db::ReadDb>();
                                let f = filter();
                                spawn(async move {
                                    let all = read_db
                                        .tracks_page(&f, Page { offset: 0, limit: u32::MAX })
                                        .await
                                        .unwrap_or_default();
                                    queue.set(all);
                                    ctrl.play_track(idx);
                                });
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
                    on_close: move |_| {
                        show_playlist_modal.set(false);
                        is_selection_mode.set(false);
                        selected_tracks.write().clear();
                    },
                    on_add_to_playlist: move |playlist_id: String| {
                        let paths: Vec<reader::TrackId> = if is_selection_mode() {
                            selected_tracks.read().iter().cloned().collect()
                        } else {
                            selected_track_for_playlist.read().iter().cloned().collect()
                        };
                        let refs: Vec<String> = paths.iter().map(|p| p.key().into_owned()).collect();
                        if !refs.is_empty() {
                            let s = active_source.peek().clone();
                            spawn(async move {
                                if s.add_to_playlist(&playlist_id, &refs).await.is_ok() {
                                    gens.bump(Table::Playlists);
                                }
                            });
                        }
                        show_playlist_modal.set(false);
                        active_menu_track.set(None);
                        is_selection_mode.set(false);
                        selected_tracks.write().clear();
                    },
                    on_create_playlist: move |name: String| {
                        let paths: Vec<reader::TrackId> = if is_selection_mode() {
                            selected_tracks.read().iter().cloned().collect()
                        } else {
                            selected_track_for_playlist.read().iter().cloned().collect()
                        };
                        let refs: Vec<String> = paths.iter().map(|p| p.key().into_owned()).collect();
                        if !refs.is_empty() {
                            let s = active_source.peek().clone();
                            spawn(async move {
                                if s.create_playlist(&name, &refs).await.is_ok() {
                                    gens.bump(Table::Playlists);
                                }
                            });
                        }
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
                        let Some(path) = track.id.local_path().map(|p| p.to_path_buf()) else {
                            return;
                        };
                        match reader::write_tags(&path, &edits) {
                            Ok(()) => {
                                let mut t = track.clone();
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
                                let s = consume_context::<::server::source::LocalHandle>().0.clone();
                                spawn(async move {
                                    if s.upsert_tracks(&[t]).await.is_ok() {
                                        gens.bump(Table::Tracks);
                                    }
                                });
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
                    show_delete: caps().delete_from_disk,
                    on_add_to_queue: move |_| {
                        let selected = selected_tracks.read().clone();
                        if selected.is_empty() {
                            return;
                        }
                        let read_db = consume_context::<db::ReadDb>();
                        let f = filter();
                        spawn(async move {
                            let total = read_db.tracks_count(&f).await.unwrap_or(0);
                            let tracks: Vec<_> = read_db
                                .tracks_page(&f, Page { offset: 0, limit: total })
                                .await
                                .unwrap_or_default()
                                .into_iter()
                                .filter(|t| selected.contains(&t.id))
                                .collect();
                            if !tracks.is_empty() {
                                ctrl.add_to_queue(tracks);
                            }
                        });
                        selected_tracks.write().clear();
                        is_selection_mode.set(false);
                    },
                    on_add_to_playlist: move |_| show_playlist_modal.set(true),
                    on_delete: move |_| {
                        if caps().delete_from_disk {
                            let paths: Vec<_> = selected_tracks.read().iter().cloned().collect();
                            let mut keys = Vec::new();
                            for id in paths {
                                let Some(path) = id.local_path() else {
                                    continue;
                                };
                                if std::fs::remove_file(path).is_ok() {
                                    keys.push(id.key().into_owned());
                                }
                            }
                            if !keys.is_empty() {
                                let s = consume_context::<::server::source::LocalHandle>().0.clone();
                                spawn(async move {
                                    if s.delete_tracks(&keys).await.is_ok() {
                                        gens.bump(Table::Tracks);
                                    }
                                });
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
                    title: if caps().scan_folders { i18n::t("rescan_library").to_string() } else { i18n::t("refresh_music_library").to_string() },
                    onclick: move |_| {
                        if caps().scan_folders {
                            on_rescan.call(());
                        } else if caps().sync {
                            sync_server();
                        }
                    },
                    i { class: "fa-solid fa-rotate" }
                }
            }

            div {
                class: if cfg!(target_os = "android") { "grid grid-cols-4 gap-2 mb-4" } else { "grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4 mb-12" },
                {
                    let album_count = albums_res
                        .read()
                        .clone()
                        .unwrap_or_default()
                        .iter()
                        .map(|a| a.title.to_lowercase())
                        .collect::<HashSet<_>>()
                        .len();
                    let artist_count = artists_res.read().as_ref().map(|a| a.len()).unwrap_or(0);
                    let playlist_count = playlists_res
                        .read()
                        .as_ref()
                        .map(|s| s.playlists.len())
                        .unwrap_or(0);
                    rsx! {
                        StatCard { label: i18n::t("tracks").to_string(), value: "{total_tracks}", icon: "fa-music" }
                        StatCard { label: i18n::t("albums").to_string(), value: "{album_count}", icon: "fa-compact-disc" }
                        StatCard { label: i18n::t("artists").to_string(), value: "{artist_count}", icon: "fa-user" }
                        StatCard { label: i18n::t("playlists").to_string(), value: "{playlist_count}", icon: "fa-list" }
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
                            if all_selected {
                                selected_tracks.write().clear();
                                is_selection_mode.set(false);
                            } else {
                                let read_db = consume_context::<db::ReadDb>();
                                let f = filter();
                                spawn(async move {
                                    let total = read_db.tracks_count(&f).await.unwrap_or(0);
                                    let tracks = read_db
                                        .tracks_page(&f, Page { offset: 0, limit: total })
                                        .await
                                        .unwrap_or_default();
                                    selected_tracks
                                        .set(tracks.into_iter().map(|track| track.id).collect());
                                    is_selection_mode.set(true);
                                });
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
                    if window.total.read().is_none() || *is_loading.read() {
                        div { class: "flex items-center justify-center py-12",
                            i { class: "fa-solid fa-spinner fa-spin text-3xl text-white/20" }
                        }
                    } else {
                        p { class: "text-slate-500 italic", "{i18n::t(\"no_tracks_found\")}" }
                    }
                } else {
                    {tracks_nodes.into_iter()}
                    if *is_loading.read() {
                        div { class: "flex items-center justify-center py-4",
                            i { class: "fa-solid fa-spinner fa-spin text-xl text-white/20" }
                        }
                    }
                }
            }
        }
    }
}
