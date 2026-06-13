use crate::server::download_manager::{DownloadQueue, DownloadStatus, queue_downloads};
use components::NavigationController;
use components::dots_menu::{DotsMenu, MenuAction};
use components::playlist_modal::PlaylistModal;
use components::selection_bar::SelectionBar;
use components::track_row::TrackRow;
use components::virtual_scroll::{VirtualScrollView, use_virtual_scroll};
use config::{AppConfig, MusicService, UiStyle};
use db::Source;
use dioxus::prelude::*;
use hooks::db_reactivity::Table;
use hooks::use_db_queries::{use_album, use_albums, use_tracks_by_keys};
use std::collections::HashSet;
use std::path::PathBuf;

#[component]
pub fn JellyfinAlbum(
    config: Signal<AppConfig>,
    mut album_id: Signal<String>,
    mut queue: Signal<Vec<reader::models::Track>>,
    mut open_album_menu: Signal<Option<String>>,
    mut show_album_playlist_modal: Signal<bool>,
    mut pending_album_id_for_playlist: Signal<Option<String>>,
) -> Element {
    let is_offline = use_context::<Signal<bool>>();
    let gens = hooks::db_reactivity::use_generations();
    let active_server_id = use_memo(move || {
        let c = config.read();
        c.active_source
            .server_id()
            .map(String::from)
            .or_else(|| c.server.as_ref().and_then(|s| s.id.clone()))
            .unwrap_or_default()
    });
    let server_source = use_memo(move || Source::Server(active_server_id()));
    let albums_res = use_albums(server_source);
    let offline_keys = use_memo(move || -> Vec<String> {
        if !*is_offline.read() {
            return Vec::new();
        }
        config
            .read()
            .offline_tracks
            .iter()
            .filter(|(_, path)| std::path::Path::new(path).exists())
            .map(|(id, _)| id.clone())
            .collect()
    });
    let offline_tracks_res = use_tracks_by_keys(server_source, offline_keys);
    let jellyfin_albums = use_memo(move || {
        let conf = config.read();

        let mut albums = albums_res.read().clone().unwrap_or_default();
        albums.sort_by(|a, b| {
            a.title
                .trim()
                .to_lowercase()
                .cmp(&b.title.trim().to_lowercase())
        });

        let mut unique_albums = Vec::new();
        let mut seen_titles = std::collections::HashSet::new();

        let offline = *is_offline.read();
        let downloaded_album_ids: std::collections::HashSet<String> = if offline {
            offline_tracks_res
                .read()
                .clone()
                .unwrap_or_default()
                .iter()
                .map(|t| t.album_id.clone())
                .collect()
        } else {
            std::collections::HashSet::new()
        };

        for album in albums {
            if offline && !downloaded_album_ids.contains(&album.id) {
                continue;
            }
            if seen_titles.insert(album.title.trim().to_lowercase()) {
                unique_albums.push(album);
            }
        }

        unique_albums
            .into_iter()
            .map(|album| {
                let cover_url = if let Some(server) = &conf.server {
                    utils::map_cover_url(album.cover_path.as_ref().and_then(|cover_path| {
                        let path_str = cover_path.to_string_lossy();
                        utils::jellyfin_image::jellyfin_image_url_from_path(
                            &path_str,
                            &server.url,
                            server.access_token.as_deref(),
                            360,
                            80,
                        )
                    }))
                } else {
                    None
                };
                (
                    album.id.clone(),
                    album.title.clone(),
                    album.artist.clone(),
                    cover_url,
                )
            })
            .collect::<Vec<_>>()
    });

    let mut ctrl = use_context::<hooks::use_player_controller::PlayerController>();

    let add_all_to_queue_text = i18n::t("add_all_to_queue").to_string();
    let add_all_to_playlist_text = i18n::t("add_all_to_playlist").to_string();
    let remove_from_cache_text = i18n::t("remove_from_cache").to_string();

    let album_menu_actions = vec![
        MenuAction::new(add_all_to_queue_text.as_str(), "fa-solid fa-list-ul"),
        MenuAction::new(add_all_to_playlist_text.as_str(), "fa-solid fa-plus"),
        MenuAction::new(remove_from_cache_text.as_str(), "fa-solid fa-trash").destructive(),
    ];

    rsx! {
        div {
            if jellyfin_albums().is_empty() {
                p { class: "text-slate-500", "{i18n::t(\"no_albums_found\")}" }
            } else {
                div { class: "grid grid-cols-[repeat(auto-fill,minmax(180px,1fr))] gap-6",
                    for (album_id_val, album_title, artist, cover_url) in jellyfin_albums() {
                        {
                            let id_for_nav    = album_id_val.clone();
                            let id_for_menu   = album_id_val.clone();
                            let id_for_action = album_id_val.clone();
                            let is_open = open_album_menu.read().as_deref() == Some(&album_id_val);
                            rsx! {
                                div {
                                    key: "{album_id_val}",
                                    class: if is_open { "group relative z-50 p-4 bg-white/5 rounded-xl hover:bg-white/10 transition-colors" } else { "group relative p-4 bg-white/5 rounded-xl hover:bg-white/10 transition-colors" },
                                    style: if is_open { "content-visibility: visible; contain: none; contain-intrinsic-size: 0 230px;" } else { "content-visibility: auto; contain-intrinsic-size: 0 230px;" },
                                    oncontextmenu: {
                                        let id = id_for_menu.clone();
                                        move |evt| {
                                            evt.prevent_default();
                                            open_album_menu.set(Some(id.clone()));
                                        }
                                    },

                                    div {
                                        class: "cursor-pointer",
                                        onclick: move |_| {
                                            album_id.set(id_for_nav.clone());
                                        },
                                        div {
                                            class: "aspect-square rounded-lg bg-stone-800 mb-3 overflow-hidden relative",
                                            style: "-webkit-user-drag: none;",
                                            ondragstart: move |evt| evt.prevent_default(),
                                            if let Some(url) = &cover_url {
                                                img { src: "{url}", class: "w-full h-full object-cover", decoding: "async", loading: "lazy", draggable: "false", ondragstart: move |evt| evt.prevent_default() }
                                            } else {
                                                div { class: "w-full h-full flex items-center justify-center",
                                                    i { class: "fa-solid fa-compact-disc text-4xl text-white/20" }
                                                }
                                            }
                                        }
                                        h3 { class: "text-white font-medium truncate", "{album_title}" }
                                        p { class: "text-sm text-stone-400 truncate", "{artist}" }
                                    }

                                    div {
                                        class: "absolute bottom-3 right-3",
                                        DotsMenu {
                                            actions: album_menu_actions.clone(),
                                            is_open,
                                            on_open: {
                                                let id = id_for_menu.clone();
                                                move |_| open_album_menu.set(Some(id.clone()))
                                            },
                                            on_close: move |_| open_album_menu.set(None),
                                            button_class: "opacity-0 group-hover:opacity-100 focus:opacity-100 bg-black/40".to_string(),
                                            anchor: "right".to_string(),
                                            on_action: {
                                                let id = id_for_action.clone();
                                                move |idx: usize| {
                                                    open_album_menu.set(None);
                                                    match idx {
                                                        0 => {
                                                            let aid = id.clone();
                                                            let s = server_source.peek().clone();
                                                            let db = consume_context::<db::Db>();
                                                            spawn(async move {
                                                                let tracks_for_queue = db
                                                                    .album_tracks(&s, &aid)
                                                                    .await
                                                                    .unwrap_or_default();
                                                                ctrl.add_to_queue(tracks_for_queue);
                                                            });
                                                        }
                                                        1 => {
                                                            pending_album_id_for_playlist.set(Some(id.clone()));
                                                            show_album_playlist_modal.set(true);
                                                        }
                                                        2 => {
                                                            let albums = albums_res.read().clone().unwrap_or_default();
                                                            let title = albums.iter()
                                                                .find(|a| a.id == id)
                                                                .map(|a| a.title.clone());
                                                            if let Some(t) = title {
                                                                let ids: Vec<String> = albums
                                                                    .iter()
                                                                    .filter(|a| a.title == t)
                                                                    .map(|a| a.id.clone())
                                                                    .collect();
                                                                let sid = active_server_id();
                                                                let db = consume_context::<db::Db>();
                                                                spawn(async move {
                                                                    let source = Source::Server(sid);
                                                                    for aid in &ids {
                                                                        let _ = db.delete_album(&source, aid).await;
                                                                    }
                                                                    gens.bump(Table::Tracks);
                                                                    gens.bump(Table::Albums);
                                                                });
                                                            }
                                                        }
                                                        _ => {}
                                                    }
                                                }
                                            },
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub fn JellyfinAlbumDetails(
    album_jellyfin_id: String,
    config: Signal<AppConfig>,
    mut queue: Signal<Vec<reader::models::Track>>,
    on_close: EventHandler<()>,
) -> Element {
    let is_offline = use_context::<Signal<bool>>();
    let nav_ctrl = use_context::<NavigationController>();
    let mut ctrl = use_context::<hooks::use_player_controller::PlayerController>();
    let mut active_menu_track = use_signal(|| None::<PathBuf>);
    let mut show_playlist_modal = use_signal(|| false);
    let mut selected_track_for_playlist = use_signal(|| None::<PathBuf>);

    // Multi-selection state
    let mut is_selection_mode = use_signal(|| false);
    let mut selected_tracks = use_signal(HashSet::<PathBuf>::new);
    let download_queue = use_context::<Signal<DownloadQueue>>();

    let album_id_memo = use_memo(use_reactive!(|album_jellyfin_id| album_jellyfin_id));

    let scroll_stat = use_signal(|| 0.0_f64);
    let container_height = use_signal(|| 0.0_f64);
    const ITEM_HEIGHT: f64 = 60.0;

    let active_server_id = use_memo(move || {
        let c = config.read();
        c.active_source
            .server_id()
            .map(String::from)
            .or_else(|| c.server.as_ref().and_then(|s| s.id.clone()))
            .unwrap_or_default()
    });
    let server_source = use_memo(move || Source::Server(active_server_id()));
    let album_res = use_album(server_source, album_id_memo);
    let albums_res = use_albums(server_source);

    let album_info = use_memo(move || album_res.read().clone().flatten());

    // The grid dedupes albums by title, so this view shows every same-titled
    // album's tracks — resolve the title to its album ids, then query each.
    let matching_album_ids = use_memo(move || -> Vec<String> {
        let Some(info) = album_info() else {
            return Vec::new();
        };
        let ids: Vec<String> = albums_res
            .read()
            .clone()
            .unwrap_or_default()
            .into_iter()
            .filter(|a| a.title == info.title)
            .map(|a| a.id)
            .collect();
        if ids.is_empty() { vec![info.id] } else { ids }
    });
    let gens = hooks::db_reactivity::use_generations();
    let db = use_context::<db::Db>();
    let album_tracks_res = use_resource(move || {
        let _ = gens.generation(Table::Tracks);
        let (db, s, ids) = (db.clone(), server_source(), matching_album_ids());
        async move {
            let mut out = Vec::new();
            for id in &ids {
                out.extend(db.album_tracks(&s, id).await.unwrap_or_default());
            }
            out
        }
    });

    let album_tracks = use_memo(move || {
        let all_tracks = album_tracks_res.read().clone().unwrap_or_default();
        let conf = config.read();
        let offline = *is_offline.read();

        let mut tracks: Vec<_> = all_tracks
            .iter()
            .filter(|t| {
                if !offline {
                    return true;
                }
                let id = t.id.key();
                conf.offline_tracks.contains_key(id.as_ref())
            })
            .map(|t| {
                let cover_url = if let Some(server) = &conf.server {
                    utils::map_cover_url(utils::jellyfin_image::resolve_track_cover(
                        t.cover.as_deref(),
                        &t.id.key(),
                        &t.album_id,
                        &server.url,
                        server.access_token.as_deref(),
                        80,
                        80,
                    ))
                } else {
                    None
                };
                (t.clone(), cover_url)
            })
            .collect();

        tracks.sort_by(|a, b| {
            let disc_cmp =
                a.0.disc_number
                    .unwrap_or(1)
                    .cmp(&b.0.disc_number.unwrap_or(1));
            if disc_cmp == std::cmp::Ordering::Equal {
                a.0.track_number
                    .unwrap_or(0)
                    .cmp(&b.0.track_number.unwrap_or(0))
            } else {
                disc_cmp
            }
        });

        tracks
    });

    let album = album_info();
    let album_title = album.as_ref().map(|a| a.title.clone()).unwrap_or_default();
    let artist = album.as_ref().map(|a| a.artist.clone()).unwrap_or_default();

    let total_seconds: u64 = album_tracks().iter().map(|(t, _)| t.duration).sum();
    let duration_min = total_seconds / 60;

    let songs_text = i18n::t("songs").to_string();
    let min_text = i18n::t("min").to_string();

    let cover_url = {
        let conf = config.read();
        if let Some(server) = &conf.server {
            utils::map_cover_url(album.as_ref().and_then(|a| {
                a.cover_path.as_ref().and_then(|cover_path| {
                    let path_str = cover_path.to_string_lossy();
                    utils::jellyfin_image::jellyfin_image_url_from_path(
                        &path_str,
                        &server.url,
                        server.access_token.as_deref(),
                        512,
                        90,
                    )
                })
            }))
        } else {
            None
        }
    };

    let currently_playing_idx: Option<usize> = {
        let queue = ctrl.queue.read();
        let current_index = *ctrl.current_queue_index.read();
        if let Some(q_idx) = ctrl.get_queue_index(current_index) {
            let tracks: Vec<_> = album_tracks().into_iter().map(|(t, _)| t).collect();
            if queue.len() == tracks.len()
                && queue.iter().zip(tracks.iter()).all(|(q, t)| q.id == t.id)
            {
                Some(q_idx)
            } else {
                None
            }
        } else {
            None
        }
    };

    let is_modern = config.read().ui_style == UiStyle::Modern;

    let scroll_info = use_virtual_scroll(
        *scroll_stat.read(),
        *container_height.read(),
        album_tracks().len(),
        ITEM_HEIGHT,
    );

    rsx! {
        div {
            class: "w-full max-w-[1600px] mx-auto select-none flex-1 min-h-0 flex flex-col",

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
                    }
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
                        let tracks: Vec<_> = album_tracks()
                            .iter()
                            .filter(|(t, _)| selected.contains(&t.id.uid_path()))
                            .map(|(track, _)| track.clone())
                            .collect();
                        if !album_tracks.is_empty() {
                            ctrl.add_to_queue(tracks);
                        }
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
                    }
                }
            }

            if !cfg!(target_os = "android") {
                div { class: "shrink-0",
                    div { class: "flex items-center justify-between mb-8",
                        button {
                            class: "flex items-center gap-2 text-slate-400 hover:text-white transition-colors",
                            onclick: move |_| on_close.call(()),
                            i { class: "fa-solid fa-arrow-left" }
                            "{i18n::t(\"back_to_albums\")}"
                        }
                    }
                }
            }

            if is_modern {
                div { class: "flex items-end gap-6 mb-8 shrink-0",
                    div {
                        class: "w-44 h-44 rounded-2xl overflow-hidden shrink-0 shadow-2xl bg-white/5",
                        style: "box-shadow: 0 20px 60px rgba(0,0,0,0.6);",
                        if let Some(url) = &cover_url {
                            img { src: "{url}", class: "w-full h-full object-cover" }
                        } else {
                            div { class: "w-full h-full flex items-center justify-center",
                                i { class: "fa-solid fa-music text-4xl", style: "color: rgba(255,255,255,0.15);" }
                            }
                        }
                    }
                    div { class: "flex flex-col gap-1 pb-1 min-w-0",
                        if !artist.is_empty() {
                            button {
                                class: "text-xs font-bold tracking-widest uppercase mb-1 text-left cursor-pointer hover:underline transition-colors",
                                style: "color: rgba(255,255,255,0.45);",
                                onclick: {
                                    let artist = artist.clone();
                                    move |_| nav_ctrl.navigate_to_artist(artist.clone())
                                },
                                "{artist}"
                            }
                        }
                        h1 { class: "text-4xl font-bold text-white truncate mb-1", "{album_title}" }
                        p {
                            class: "text-sm mb-3",
                            style: "color: rgba(255,255,255,0.45);",
                            "{album_tracks().len()} {songs_text} · {duration_min} {min_text}"
                        }
                        div { class: "flex items-center gap-2 flex-wrap",
                            if !album_tracks().is_empty() {
                                button {
                                    class: "inline-flex items-center justify-center gap-2 h-9 px-5 rounded-full text-sm font-semibold text-white transition-opacity hover:opacity-90 active:scale-95",
                                    style: "background: var(--color-indigo-500);",
                                    onclick: {
                                        let tracks_for_play: Vec<reader::models::Track> = album_tracks().iter().map(|(t, _)| t.clone()).collect();
                                        move |_| {
                                            let is_shuffle = *ctrl.shuffle.peek();
                                            if is_shuffle {
                                                ctrl.play_queue_shuffled(tracks_for_play.clone());
                                            } else {
                                                ctrl.play_queue_linear(tracks_for_play.clone());
                                            }
                                        }
                                    },
                                    i { class: "fa-solid fa-play text-xs" }
                                    "{i18n::t(\"play\")}"
                                }
                                button {
                                    class: "inline-flex items-center justify-center gap-2 h-9 px-5 rounded-full text-sm font-semibold text-white transition-opacity hover:opacity-90 active:scale-95",
                                    style: if *ctrl.shuffle.read() {
                                        "background: var(--color-indigo-500);"
                                    } else {
                                        "background: color-mix(in oklab, var(--color-indigo-500) 25%, transparent); border: 1px solid color-mix(in oklab, var(--color-indigo-500) 40%, transparent);"
                                    },
                                    onclick: {
                                        let tracks_for_shuffle: Vec<reader::models::Track> = album_tracks().iter().map(|(t, _)| t.clone()).collect();
                                        move |_| {
                                            ctrl.toggle_shuffle();
                                            ctrl.play_queue_shuffled(tracks_for_shuffle.clone());
                                        }
                                    },
                                    i { class: "fa-solid fa-shuffle text-xs" }
                                    "{i18n::t(\"shuffle\")}"
                                }
                                {
                                    let is_album_dl = {
                                        let q = download_queue.read();
                                        album_tracks().iter().any(|(t, _)| {
                                            let id = t.id.key();
                                            q.items.iter().any(|i| i.id == id.as_ref() && matches!(i.status, DownloadStatus::Queued | DownloadStatus::Downloading))
                                        })
                                    };
                                    let all_downloaded = !album_tracks().is_empty() && album_tracks().iter().all(|(t, _)| {
                                        let id = t.id.key();
                                        if let Some(path_str) = config.read().offline_tracks.get(id.as_ref()) {
                                            std::path::Path::new(path_str).exists()
                                        } else {
                                            false
                                        }
                                    });
                                    rsx! {
                                        button {
                                            class: "inline-flex items-center justify-center h-9 w-9 rounded-full text-sm font-medium transition-colors hover:bg-white/10",
                                            style: "color: rgba(255,255,255,0.6); border: 1px solid rgba(255,255,255,0.12);",
                                            title: if all_downloaded { "Remove downloads" } else { "Download album for offline playback" },
                                            disabled: is_album_dl,
                                            onclick: move |_| {
                                                let ids_only: Vec<String> = album_tracks()
                                                    .iter()
                                                    .filter_map(|(t, _)| {
                                                        let k = t.id.key();
                                                        (!k.is_empty()).then(|| k.to_string())
                                                    })
                                                    .collect();
                                                if all_downloaded {
                                                    crate::server::download_manager::delete_downloads(ids_only, config, download_queue);
                                                } else {
                                                    let requests: Vec<(String, String, String)> = album_tracks()
                                                        .iter()
                                                        .filter_map(|(t, _)| {
                                                            let k = t.id.key();
                                                            (!k.is_empty()).then(|| (
                                                                k.to_string(),
                                                                t.title.clone(),
                                                                t.artist.clone(),
                                                            ))
                                                        })
                                                        .collect();
                                                    queue_downloads(requests, config, download_queue);
                                                }
                                            },
                                            if is_album_dl {
                                                i { class: "fa-solid fa-spinner fa-spin text-xs" }
                                            } else if all_downloaded {
                                                i { class: "fa-solid fa-trash text-xs" }
                                            } else {
                                                i { class: "fa-solid fa-download text-xs" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                div {
                    class: "flex flex-col md:flex-row items-end gap-8 mb-12 shrink-0",
                    div { class: "w-64 h-64 rounded-xl bg-stone-800 overflow-hidden relative flex-shrink-0",
                        if let Some(url) = &cover_url {
                            img { src: "{url}", class: "w-full h-full object-cover" }
                        } else {
                            div { class: "w-full h-full flex flex-col items-center justify-center text-white/20",
                                i { class: "fa-solid fa-music text-6xl mb-4" }
                            }
                        }
                    }
                    div { class: "flex-1",
                        if !artist.is_empty() {
                            button {
                                class: "text-sm font-bold tracking-widest text-white/60 uppercase mb-2 text-left cursor-pointer hover:underline hover:text-white transition-colors",
                                onclick: {
                                    let artist = artist.clone();
                                    move |_| nav_ctrl.navigate_to_artist(artist.clone())
                                },
                                "{artist}"
                            }
                        }
                        h1 { class: "text-5xl md:text-7xl font-bold text-white mb-6", "{album_title}" }
                        div { class: "flex items-center gap-6 text-slate-400",
                            p { "{album_tracks().len()} {songs_text}" }
                            span { "•" }
                            p { "{duration_min} {min_text}" }
                        }
                    }
                    div { class: "flex items-center gap-4",
                        if !album_tracks().is_empty() {
                            button {
                                class: format!("w-14 h-14 rounded-full flex items-center justify-center {}", if *ctrl.shuffle.read() { "text-white" } else { "text-slate-400 hover:text-white" }),
                                title: if *ctrl.shuffle.read() {
                                    i18n::t("shuffle_on").to_string()
                                } else {
                                    i18n::t("shuffle_off").to_string()
                                },
                                onclick: move |_| ctrl.toggle_shuffle(),
                                i { class: "fa-solid fa-shuffle text-xl ml-1" }
                            }
                            button {
                                class: "w-14 h-14 rounded-full bg-indigo-500 hover:bg-indigo-400 text-black flex items-center justify-center transition-transform hover:scale-105",
                                onclick: {
                                    let tracks_for_play: Vec<reader::models::Track> = album_tracks().iter().map(|(t, _)| t.clone()).collect();
                                    move |_| {
                                        let is_shuffle = *ctrl.shuffle.peek();
                                        if is_shuffle {
                                            ctrl.play_queue_shuffled(tracks_for_play.clone());
                                        } else {
                                            ctrl.play_queue_linear(tracks_for_play.clone());
                                        }
                                    }
                                  },
                                i { class: "fa-solid fa-play text-xl ml-1" }
                            }
                            {
                                let is_album_dl = {
                                    let q = download_queue.read();
                                    album_tracks().iter().any(|(t, _)| {
                                        let id = t.id.key();
                                        q.items.iter().any(|i| i.id == id.as_ref() && matches!(i.status, DownloadStatus::Queued | DownloadStatus::Downloading))
                                    })
                                };
                                let all_downloaded = !album_tracks().is_empty() && album_tracks().iter().all(|(t, _)| {
                                    let id = t.id.key();
                                    if let Some(path_str) = config.read().offline_tracks.get(id.as_ref()) {
                                        std::path::Path::new(path_str).exists()
                                    } else {
                                        false
                                    }
                                });
                                rsx! {
                                    button {
                                        class: "w-12 h-12 rounded-full border border-white/20 hover:border-white/40 text-white/70 hover:text-white flex items-center justify-center transition-colors",
                                        title: if all_downloaded { "Remove downloads" } else { "Download album for offline playback" },
                                        disabled: is_album_dl,
                                        onclick: move |_| {
                                            let ids_only: Vec<String> = album_tracks()
                                                .iter()
                                                .filter_map(|(t, _)| {
                                                    let k = t.id.key();
                                                    (!k.is_empty()).then(|| k.to_string())
                                                })
                                                .collect();
                                            if all_downloaded {
                                                crate::server::download_manager::delete_downloads(ids_only, config, download_queue);
                                            } else {
                                                let requests: Vec<(String, String, String)> = album_tracks()
                                                    .iter()
                                                    .filter_map(|(t, _)| {
                                                        let k = t.id.key();
                                                        (!k.is_empty()).then(|| (
                                                            k.to_string(),
                                                            t.title.clone(),
                                                            t.artist.clone(),
                                                        ))
                                                    })
                                                    .collect();
                                                queue_downloads(requests, config, download_queue);
                                            }
                                        },
                                        if is_album_dl {
                                            i { class: "fa-solid fa-spinner fa-spin" }
                                        } else if all_downloaded {
                                            i { class: "fa-solid fa-trash" }
                                        } else {
                                            i { class: "fa-solid fa-download" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            div {
                if album_tracks().is_empty() {
                    div { class: "py-12 flex flex-col items-center justify-center text-slate-600",
                        i { class: "fa-regular fa-folder-open text-4xl mb-4" }
                        p { class: "text-lg", "{i18n::t(\"no_songs_here\")}" }
                    }
                } else {
                    if is_modern {
                        div {
                            class: "grid px-3 py-2 text-[10px] font-bold uppercase tracking-widest border-b mb-1",
                            style: "grid-template-columns: 40px 1fr 180px 56px 40px; color: rgba(255,255,255,0.25); border-color: rgba(255,255,255,0.06);",
                            div {}
                            div { "{i18n::t(\"title\")}" }
                            div { "{i18n::t(\"artist\")}" }
                            div { class: "text-right pr-2", i { class: "fa-regular fa-clock" } }
                            div {}
                        }
                    } else {
                        div { class: "grid grid-cols-[auto_1fr_1fr_auto_auto] gap-4 px-2 py-2 border-b border-white/5 text-sm font-medium text-slate-500 mb-2 uppercase tracking-wider",
                            div { class: "flex items-center w-24 shrink-0",
                                div { class: "mr-4 flex items-center justify-center w-6 h-6 shrink-0",
                                    button {
                                        class: if !album_tracks().is_empty() && album_tracks().iter().all(|(track, _)| selected_tracks.read().contains(&track.id.uid_path())) {
                                            "w-4 h-4 rounded border border-indigo-400 bg-indigo-500 text-white flex items-center justify-center transition-colors"
                                        } else {
                                            "w-4 h-4 rounded border border-white/20 bg-white/5 hover:border-white/50 transition-colors"
                                        },
                                        aria_label: "Select all tracks",
                                        onclick: move |_| {
                                            let tracks = album_tracks();
                                            let all_selected = !tracks.is_empty() && tracks.iter().all(|(track, _)| selected_tracks.read().contains(&track.id.uid_path()));
                                            if all_selected {
                                                selected_tracks.write().clear();
                                                is_selection_mode.set(false);
                                            } else {
                                                selected_tracks.set(tracks.into_iter().map(|(track, _)| track.id.uid_path()).collect());
                                                is_selection_mode.set(true);
                                            }
                                        },
                                        if !album_tracks().is_empty() && album_tracks().iter().all(|(track, _)| selected_tracks.read().contains(&track.id.uid_path())) {
                                            i { class: "fa-solid fa-check", style: "font-size: 9px;" }
                                        }
                                    }
                                }
                            }
                            div { "{i18n::t(\"title\")}" }
                            div { "{i18n::t(\"album\")}" }
                        }
                    }
                }
                div { class: "flex-1 min-h-0 w-full flex flex-col overflow-hidden",
                    VirtualScrollView {
                        id: "server-album-scroll".to_string(),
                        class: "flex-1 min-h-0 overflow-y-auto pb-20".to_string(),
                        scroll_stat,
                        container_height,
                        item_height: ITEM_HEIGHT,
                        saved_scroll: 0.0,
                        top_pad: scroll_info.top_pad,
                        bottom_pad: scroll_info.bottom_pad,
                        for (idx, (track, track_cover_url)) in album_tracks().into_iter().enumerate().skip(scroll_info.start_index).take(scroll_info.items_to_render) {
                            {
                            let track_key = track.id.uid();
                            let track_menu = track.clone();
                            let track_add  = track.clone();
                            let track_queue = track.clone();
                            let track_path = track.id.uid_path();
                            let track_select = track.id.uid_path();
                            let is_menu_open = active_menu_track.read().as_ref() == Some(&track.id.uid_path());
                            let album_queue: Vec<reader::models::Track> =
                                album_tracks().iter().map(|(t, _)| t.clone()).collect();

                            let item_id: String = track.id.key().to_string();
                            let is_downloaded = if let Some(path_str) = config.read().offline_tracks.get(&item_id) {
                                std::path::Path::new(path_str).exists()
                            } else {
                                false
                            };
                            let is_downloading = download_queue.read().items.iter().any(|i| i.id == item_id && matches!(i.status, DownloadStatus::Queued | DownloadStatus::Downloading));
                            let item_id_dl = item_id.clone();
                            let track_title = track.title.clone();
                            let track_artist = track.artist.clone();

                            rsx! {
                                TrackRow {
                                    key: "{track_key}",
                                    track: track.clone(),
                                    cover_url: track_cover_url,
                                    row_num: Some(idx + 1),
                                    is_menu_open,
                                    is_album: true,
                                    is_currently_playing: currently_playing_idx == Some(idx),
                                    is_selection_mode: is_selection_mode(),
                                    is_selected: selected_tracks.read().contains(&track_path),
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
                                    on_play: move |_| {
                                        queue.set(album_queue.clone());
                                        ctrl.play_track(idx);
                                    },
                                    on_delete: move |_| active_menu_track.set(None),
                                    hide_delete: true,
                                    on_download: move |_| {
                                        active_menu_track.set(None);
                                        if is_downloaded {
                                            crate::server::download_manager::delete_downloads(
                                                vec![item_id_dl.clone()],
                                                config,
                                                download_queue,
                                            );
                                        } else {
                                            queue_downloads(
                                                vec![(item_id_dl.clone(), track_title.clone(), track_artist.clone())],
                                                config,
                                                download_queue,
                                            );
                                        }
                                    },
                                }
                            }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub fn ServerAlbum(
    config: Signal<AppConfig>,
    album_id: Signal<String>,
    queue: Signal<Vec<reader::models::Track>>,
    open_album_menu: Signal<Option<String>>,
    show_album_playlist_modal: Signal<bool>,
    pending_album_id_for_playlist: Signal<Option<String>>,
) -> Element {
    let service = config
        .read()
        .active_service()
        .unwrap_or(MusicService::Jellyfin);

    match service {
        MusicService::Jellyfin => rsx! {
            JellyfinAlbum {
                config,
                album_id,
                queue,
                open_album_menu,
                show_album_playlist_modal,
                pending_album_id_for_playlist,
            }
        },
        MusicService::Subsonic => rsx! {
            SubsonicAlbum {
                config,
                album_id,
                queue,
                open_album_menu,
                show_album_playlist_modal,
                pending_album_id_for_playlist,
            }
        },
        MusicService::Custom | MusicService::YtMusic => rsx! {
            CustomAlbum {
                config,
                album_id,
                queue,
                open_album_menu,
                show_album_playlist_modal,
                pending_album_id_for_playlist,
            }
        },
    }
}

#[component]
pub fn SubsonicAlbum(
    config: Signal<AppConfig>,
    album_id: Signal<String>,
    queue: Signal<Vec<reader::models::Track>>,
    open_album_menu: Signal<Option<String>>,
    show_album_playlist_modal: Signal<bool>,
    pending_album_id_for_playlist: Signal<Option<String>>,
) -> Element {
    rsx! {
        JellyfinAlbum {
            config,
            album_id,
            queue,
            open_album_menu,
            show_album_playlist_modal,
            pending_album_id_for_playlist,
        }
    }
}

#[component]
pub fn CustomAlbum(
    config: Signal<AppConfig>,
    album_id: Signal<String>,
    queue: Signal<Vec<reader::models::Track>>,
    open_album_menu: Signal<Option<String>>,
    show_album_playlist_modal: Signal<bool>,
    pending_album_id_for_playlist: Signal<Option<String>>,
) -> Element {
    rsx! {
        JellyfinAlbum {
            config,
            album_id,
            queue,
            open_album_menu,
            show_album_playlist_modal,
            pending_album_id_for_playlist,
        }
    }
}

#[component]
pub fn ServerAlbumDetails(
    album_jellyfin_id: String,
    config: Signal<AppConfig>,
    queue: Signal<Vec<reader::models::Track>>,
    on_close: EventHandler<()>,
) -> Element {
    let service = config
        .read()
        .active_service()
        .unwrap_or(MusicService::Jellyfin);

    match service {
        MusicService::Jellyfin => rsx! {
            JellyfinAlbumDetails {
                album_jellyfin_id,
                config,
                queue,
                on_close,
            }
        },
        MusicService::Subsonic => rsx! {
            SubsonicAlbumDetails {
                album_jellyfin_id,
                config,
                queue,
                on_close,
            }
        },
        MusicService::Custom | MusicService::YtMusic => rsx! {
            CustomAlbumDetails {
                album_jellyfin_id,
                config,
                queue,
                on_close,
            }
        },
    }
}

#[component]
pub fn SubsonicAlbumDetails(
    album_jellyfin_id: String,
    config: Signal<AppConfig>,
    queue: Signal<Vec<reader::models::Track>>,
    on_close: EventHandler<()>,
) -> Element {
    rsx! {
        JellyfinAlbumDetails {
            album_jellyfin_id,
            config,
            queue,
            on_close,
        }
    }
}

#[component]
pub fn CustomAlbumDetails(
    album_jellyfin_id: String,
    config: Signal<AppConfig>,
    queue: Signal<Vec<reader::models::Track>>,
    on_close: EventHandler<()>,
) -> Element {
    rsx! {
        JellyfinAlbumDetails {
            album_jellyfin_id,
            config,
            queue,
            on_close,
        }
    }
}
