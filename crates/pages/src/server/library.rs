use crate::server::download_manager::{DownloadQueue, DownloadStatus, queue_downloads};
use components::playlist_modal::PlaylistModal;
use components::selection_bar::SelectionBar;
use components::stat_card::StatCard;
use components::track_row::TrackRow;
use components::virtual_scroll::{VirtualScrollView, use_virtual_scroll};
use config::{AppConfig, UiStyle};
use db::{Page, Source, TrackFilter, TrackSort};
use dioxus::prelude::*;
use hooks::use_db_queries::{use_albums, use_artists, use_playlists, use_tracks_window};
use hooks::use_player_controller::PlayerController;
use kopuz_route::Route;
use std::collections::HashSet;
use std::path::PathBuf;

const ITEM_HEIGHT: f64 = 60.0;

#[component]
pub fn JellyfinLibrary(
    mut config: Signal<AppConfig>,
    mut queue: Signal<Vec<reader::models::Track>>,
) -> Element {
    let mut ctrl = use_context::<PlayerController>();
    let mut is_loading = use_signal(|| false);
    let mut has_fetched = use_signal(|| false);
    let mut fetch_generation = use_signal(|| 0usize);
    let mut sort_order = use_signal(|| config.peek().sort_order.clone());
    let mut scroll_positions = use_context::<Signal<std::collections::HashMap<Route, f64>>>();
    let saved_scroll = scroll_positions
        .peek()
        .get(&Route::Library)
        .copied()
        .unwrap_or(0.0);
    let scroll_stat = use_signal(move || saved_scroll);
    use_effect(move || {
        let curr = sort_order.read().clone();
        if config.peek().sort_order != curr {
            config.write().sort_order = curr;
        }
    });

    let mut active_menu_track = use_signal(|| None::<PathBuf>);
    let mut show_playlist_modal = use_signal(|| false);
    let mut selected_track_for_playlist = use_signal(|| None::<PathBuf>);

    let mut is_selection_mode = use_signal(|| false);
    let mut selected_tracks = use_signal(|| HashSet::<PathBuf>::new());
    let download_queue = use_context::<Signal<DownloadQueue>>();

    let active_server_id = use_memo(move || {
        let c = config.read();
        c.active_server_id
            .clone()
            .or_else(|| c.server.as_ref().and_then(|s| s.id.clone()))
            .unwrap_or_default()
    });
    let source = use_memo(move || Source::Server(active_server_id()));
    let filter = use_memo(move || TrackFilter {
        source: Source::Server(active_server_id()),
        sort: match *sort_order.read() {
            config::SortOrder::Title => TrackSort::Title,
            config::SortOrder::Artist => TrackSort::Artist,
            config::SortOrder::Album => TrackSort::Album,
        },
        ..Default::default()
    });
    let container_height = use_signal(|| 0.0_f64);
    let mut total_items = use_signal(|| 0usize);
    let page = use_memo(move || {
        let info = use_virtual_scroll(
            *scroll_stat.read(),
            *container_height.read(),
            *total_items.read(),
            ITEM_HEIGHT,
        );
        Page {
            offset: info.start_index as u32,
            limit: info.items_to_render as u32,
        }
    });
    let window = use_tracks_window(filter, page);
    use_effect(move || {
        let total = (*window.total.read()).unwrap_or(0) as usize;
        if *total_items.peek() != total {
            total_items.set(total);
        }
    });
    let albums_res = use_albums(source);
    let artists_res = use_artists(source);
    let playlists_server_id =
        use_memo(move || Some(active_server_id()).filter(|id| !id.is_empty()));
    let playlists_res = use_playlists(playlists_server_id);

    let mut fetch_jellyfin = move || {
        has_fetched.set(true);
        is_loading.set(true);
        fetch_generation.with_mut(|g| *g += 1);
        let current_gen = *fetch_generation.peek();
        spawn(async move {
            if *fetch_generation.read() == current_gen {
                let _ =
                    crate::server::subsonic_sync::sync_server_library(config, true).await;
                if *fetch_generation.read() == current_gen {
                    is_loading.set(false);
                }
            }
        });
    };

    use_effect(move || {
        if !*has_fetched.read() {
            if let Some(total) = *window.total.read() {
                if total == 0 {
                    fetch_jellyfin();
                } else {
                    has_fetched.set(true);
                }
            }
        }
    });

    let displayed_tracks = use_memo(move || {
        let tracks = window.rows.read().clone().unwrap_or_default().rows;
        let conf = config.read();
        tracks
            .into_iter()
            .map(|t| {
                let cover_url = if let Some(server) = &conf.server {
                    utils::map_cover_url(
                        utils::jellyfin_image::resolve_track_cover(
                            t.cover.as_deref(),
                            &t.id.key(),
                            &t.album_id,
                            &server.url,
                            server.access_token.as_deref(),
                            80,
                            80,
                        ),
                    )
                } else {
                    None
                };
                (t, cover_url)
            })
            .collect::<Vec<_>>()
    });

    let total_tracks = *total_items.read();
    let is_empty = total_tracks == 0;
    let all_selected = !is_empty && selected_tracks.read().len() >= *total_items.read();
    let row_offset = window
        .rows
        .read()
        .as_ref()
        .map(|w| w.offset)
        .unwrap_or(0) as usize;

    let scroll_info = use_virtual_scroll(
        *scroll_stat.read(),
        *container_height.read(),
        total_tracks,
        ITEM_HEIGHT,
    );

    let current_track_id: Option<reader::models::TrackId> = {
        let queue = ctrl.queue.read();
        let q_idx = *ctrl.current_queue_index.read();
        if queue.len() == total_tracks {
            queue.get(q_idx).map(|t| t.id.clone())
        } else {
            None
        }
    };

    let all_tracks = displayed_tracks.read();
    let tracks_nodes = all_tracks
        .iter()
        .enumerate()
        .map(|(i, (track, cover_url))| {
            let idx = row_offset + i;
            let track_menu = track.clone();
            let track_add = track.clone();
            let track_queue = track.clone();
            let track_path = track.id.uid_path();
            let is_currently_playing = current_track_id.as_ref() == Some(&track.id);
            let track_select = track.id.uid_path();
            // Key by identity only: an index-suffixed key changes for every
            // visible row on each one-row scroll step, remounting the whole
            // window (DOM teardown + image re-decode + full repaint).
            let track_key = track.id.uid();
            let is_menu_open = active_menu_track.read().as_ref() == Some(&track.id.uid_path());
            let is_selected = selected_tracks.read().contains(&track_path);

            let item_id: String = track.id.key().to_string();
            let is_downloaded = if let Some(path_str) = config.read().offline_tracks.get(&item_id) {
                std::path::Path::new(path_str).exists()
            } else {
                false
            };
            let is_downloading = download_queue.read().items.iter().any(|i| {
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
                div { key: "{track_key}", style: "height: {ITEM_HEIGHT}px;",
                    TrackRow {
                        track: track.clone(),
                        cover_url: cover_url.clone(),
                        row_num: Some(idx + 1),
                        is_menu_open,
                        is_album: false,
                        is_currently_playing,
                        is_selection_mode: is_selection_mode(),
                        is_selected,
                        is_downloaded,
                        is_downloading,
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
                            if active_menu_track.read().as_ref() == Some(&track_menu.id.uid_path()) {
                                active_menu_track.set(None);
                            } else {
                                active_menu_track.set(Some(track_menu.id.uid_path()));
                            }
                        },
                        on_add_to_playlist: move |_| {
                            selected_track_for_playlist.set(Some(track_add.id.uid_path()));
                            show_playlist_modal.set(true);
                            active_menu_track.set(None);
                        },
                        on_queue: move |_| {
                            ctrl.add_to_queue(vec![track_queue.clone()]);
                            active_menu_track.set(None);
                        },
                        on_close_menu: move |_| active_menu_track.set(None),
                        on_delete: move |_| active_menu_track.set(None),
                        hide_delete: true,
                        on_download: move |_| {
                            if !is_downloaded {
                                active_menu_track.set(None);
                                queue_downloads(
                                    vec![(
                                        item_id_dl.clone(),
                                        track_title.clone(),
                                        track_artist.clone(),
                                    )],
                                    config,
                                    download_queue,
                                );
                            }
                        },
                        on_play: move |_| {
                            let f = filter.peek().clone();
                            let db = consume_context::<db::Db>();
                            spawn(async move {
                                let all = db
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
        });

    let is_modern = config.read().ui_style == UiStyle::Modern;

    rsx! {
        div {
            class: if cfg!(target_os = "android") { "px-3 pt-3 absolute inset-0 flex flex-col overflow-x-hidden" } else if is_modern { "px-6 pt-6 absolute inset-0 flex flex-col" } else { "px-8 pt-8 absolute inset-0 flex flex-col" },
            if *show_playlist_modal.read() {
                PlaylistModal {
                    is_jellyfin: true,
                    on_close: move |_| {
                        show_playlist_modal.set(false);
                        if is_selection_mode() {
                            is_selection_mode.set(false);
                            selected_tracks.write().clear();
                        }
                    },
                    on_add_to_playlist: move |playlist_id: String| {
                        let mut selected_paths = Vec::new();
                        if is_selection_mode() {
                            selected_paths = selected_tracks.read().iter().cloned().collect();
                        } else if let Some(path) = selected_track_for_playlist.read().clone() {
                            selected_paths.push(path);
                        }

                        if !selected_paths.is_empty() {
                            let pid = playlist_id.clone();
                            spawn(async move {
                                let Some(conn) =
                                    ::server::server_ops::ServerConn::resolve(&config.peek())
                                else {
                                    return;
                                };
                                let item_ids: Vec<String> = selected_paths
                                    .iter()
                                    .filter_map(|p| {
                                        ::server::server_ops::parse_item_id(p.to_str()?)
                                            .map(str::to_string)
                                    })
                                    .collect();
                                let _ = ::server::server_ops::add_tracks_to_playlist(
                                    &conn, &pid, &item_ids,
                                )
                                .await;
                            });
                        }
                        show_playlist_modal.set(false);
                        active_menu_track.set(None);
                        is_selection_mode.set(false);
                        selected_tracks.write().clear();
                    },
                    on_create_playlist: move |name: String| {
                        let mut selected_paths = Vec::new();
                        if is_selection_mode() {
                            selected_paths = selected_tracks.read().iter().cloned().collect();
                        } else if let Some(path) = selected_track_for_playlist.read().clone() {
                            selected_paths.push(path);
                        }

                        if !selected_paths.is_empty() {
                            let playlist_name = name.clone();
                            spawn(async move {
                                let Some(conn) =
                                    ::server::server_ops::ServerConn::resolve(&config.peek())
                                else {
                                    return;
                                };
                                let item_ids: Vec<String> = selected_paths
                                    .iter()
                                    .filter_map(|p| {
                                        ::server::server_ops::parse_item_id(p.to_str()?)
                                            .map(str::to_string)
                                    })
                                    .collect();
                                if !item_ids.is_empty() {
                                    let _ = ::server::server_ops::create_server_playlist(
                                        &conn,
                                        &playlist_name,
                                        &item_ids,
                                    )
                                    .await;
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

            if is_selection_mode() {
                SelectionBar {
                    count: selected_tracks.read().len(),
                    show_delete: false,
                    on_add_to_queue: move |_| {
                        let selected = selected_tracks.read().clone();
                        if selected.is_empty() {
                            return;
                        }
                        let keys: Vec<String> = selected
                            .iter()
                            .filter_map(|p| {
                                ::server::server_ops::parse_item_id(p.to_str()?)
                                    .map(str::to_string)
                            })
                            .collect();
                        let s = source.peek().clone();
                        let db = consume_context::<db::Db>();
                        spawn(async move {
                            let tracks = db.tracks_by_keys(&s, &keys).await.unwrap_or_default();
                            if !tracks.is_empty() {
                                ctrl.add_to_queue(tracks);
                            }
                        });
                        is_selection_mode.set(false);
                        selected_tracks.write().clear();
                    },
                    on_add_to_playlist: move |_| {
                        show_playlist_modal.set(true);
                    },
                    on_delete: move |_| {
                        is_selection_mode.set(false);
                        selected_tracks.write().clear();
                    },
                    on_cancel: move |_| {
                        is_selection_mode.set(false);
                        selected_tracks.write().clear();
                    },
                }
            }

            div { class: "flex items-center justify-between mb-6",
                if is_modern {
                    div {
                        p {
                            class: "text-[10px] font-bold tracking-widest uppercase mb-0.5",
                            style: "color: rgba(255,255,255,0.35);",
                            "{i18n::t(\"library\")}"
                        }
                        h1 { class: "text-2xl font-bold text-white", "{i18n::t(\"your_library\")}" }
                    }
                } else {
                    h1 { class: "text-3xl font-bold text-white", "{i18n::t(\"your_library\")}" }
                }
                button {
                    class: "text-white/60 hover:text-white transition-colors p-2 rounded-full hover:bg-white/10",
                    title: i18n::t("refresh_music_library").to_string(),
                    onclick: move |_| fetch_jellyfin(),
                    i { class: "fa-solid fa-rotate" }
                }
            }

            div { class: if cfg!(target_os = "android") { "grid grid-cols-4 gap-2 mb-4" } else { "grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4 mb-12" },
                {
                    let albums = albums_res.read().clone().unwrap_or_default();
                    let (artist_count, album_count) = {
                        let mut artists = HashSet::new();
                        let mut album_titles = HashSet::new();
                        for album in &albums {
                            artists.insert(album.artist.clone());
                            album_titles.insert(album.title.to_lowercase());
                        }
                        for (artist, _) in artists_res.read().clone().unwrap_or_default() {
                            artists.insert(artist);
                        }
                        (artists.len(), album_titles.len())
                    };
                    let playlist_count = playlists_res
                        .read()
                        .as_ref()
                        .map(|s| s.jellyfin_playlists.len())
                        .unwrap_or(0);
                    rsx! {
                        StatCard {
                            label: i18n::t("tracks").to_string(),
                            value: "{total_tracks}",
                            icon: "fa-music",
                        }
                        StatCard {
                            label: i18n::t("albums").to_string(),
                            value: "{album_count}",
                            icon: "fa-compact-disc",
                        }
                        StatCard {
                            label: i18n::t("artists").to_string(),
                            value: "{artist_count}",
                            icon: "fa-user",
                        }
                        StatCard {
                            label: i18n::t("playlists").to_string(),
                            value: "{playlist_count}",
                            icon: "fa-list",
                        }
                    }
                }
            }

            div { class: "flex items-center justify-between mb-4",
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
                                let db = consume_context::<db::Db>();
                                let f = filter();
                                spawn(async move {
                                    let total = db.tracks_count(&f).await.unwrap_or(0);
                                    let tracks = db
                                        .tracks_page(&f, Page { offset: 0, limit: total })
                                        .await
                                        .unwrap_or_default();
                                    selected_tracks
                                        .set(tracks.into_iter().map(|track| track.id.uid_path()).collect());
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
                div { class: "flex space-x-1 bg-white/5 border border-white/5 p-1 rounded-lg",
                    button {
                        class: if *sort_order.read() == config::SortOrder::Title {
                            "px-3 py-1 text-xs rounded-md bg-white/10 text-white font-medium transition-all"
                        } else {
                            "px-3 py-1 text-xs rounded-md text-white/40 hover:text-white/80 transition-all"
                        },
                        onclick: move |_| sort_order.set(config::SortOrder::Title),
                        "Title"
                    }
                    button {
                        class: if *sort_order.read() == config::SortOrder::Artist {
                            "px-3 py-1 text-xs rounded-md bg-white/10 text-white font-medium transition-all"
                        } else {
                            "px-3 py-1 text-xs rounded-md text-white/40 hover:text-white/80 transition-all"
                        },
                        onclick: move |_| sort_order.set(config::SortOrder::Artist),
                        "Artist"
                    }
                    button {
                        class: if *sort_order.read() == config::SortOrder::Album {
                            "px-3 py-1 text-xs rounded-md bg-white/10 text-white font-medium transition-all"
                        } else {
                            "px-3 py-1 text-xs rounded-md text-white/40 hover:text-white/80 transition-all"
                        },
                        onclick: move |_| sort_order.set(config::SortOrder::Album),
                        "Album"
                    }
                }
            }

            div {
                class: if is_modern {
                    "grid px-3 py-2 text-[10px] font-bold uppercase tracking-widest border-b mb-1"
                } else {
                    "grid gap-6 px-2 py-2 border-b border-white/5 text-sm font-medium text-slate-500 mb-2 uppercase tracking-wider"
                },
                style: if is_modern {
                    "grid-template-columns: 40px minmax(200px, 2fr) minmax(150px, 1fr) minmax(150px, 1fr) 56px 40px; color: rgba(255,255,255,0.25); border-color: rgba(255,255,255,0.06);"
                } else {
                    "grid-template-columns: 40px minmax(200px, 2fr) minmax(150px, 1fr) minmax(150px, 1fr) 64px 40px; align-items: center;"
                },
                div {}
                div { "{i18n::t(\"title\")}" }
                div { "{i18n::t(\"artist\")}" }
                div { "{i18n::t(\"album\")}" }
                div { class: "text-right pr-2",
                    i { class: "fa-regular fa-clock" }
                }
                div {}
            }

            VirtualScrollView {
                id: "server-library-scroll".to_string(),
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
                    if *is_loading.read() {
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

pub use JellyfinLibrary as ServerLibrary;
