//! Source-agnostic Album page (issue #35). One grid + one detail render any
//! source: covers resolve through the source layer, and the divergent
//! affordances (tag/cover edit + delete-from-disk for local, downloads for a
//! server) gate on the resolved source's [`Capabilities`] — no `is_server()`.

use components::dots_menu::{DotsMenu, MenuAction};
use components::playlist_modal::PlaylistModal;
use components::track_list_view::TrackListView;
use config::AppConfig;
use dioxus::prelude::*;
use hooks::db_reactivity::Table;
use hooks::use_db_queries::{
    use_active_source, use_album, use_album_tracks, use_albums, use_tracks_by_keys,
};
use std::collections::HashSet;
use std::path::PathBuf;

use crate::server::download_manager::{
    DownloadQueue, DownloadStatus, delete_downloads, queue_downloads,
};

/// One album-card menu entry, tagged so dispatch survives capability gating.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AlbumAction {
    Queue,
    Playlist,
    /// Local: delete the files + DB rows. Server: drop the cached rows (a re-sync
    /// re-adds them) — there's no remote album delete.
    Remove,
}

#[component]
pub fn Album(
    config: Signal<AppConfig>,
    album_id: Signal<String>,
    mut queue: Signal<Vec<reader::models::Track>>,
    mut current_queue_index: Signal<usize>,
) -> Element {
    let gens = hooks::db_reactivity::use_generations();
    let source = use_active_source();
    let active_source = use_context::<Signal<::server::source::ActiveSource>>();
    let caps = use_memo(move || active_source.read().capabilities());

    let open_album_menu = use_signal(|| None::<String>);
    let mut show_album_playlist_modal = use_signal(|| false);
    let pending_album_id_for_playlist = use_signal(|| None::<String>);

    let albums_res = use_albums(source);

    // First visit to a server with an empty cache → pull once.
    let mut has_fetched = use_signal(|| false);
    use_effect(move || {
        if !caps().sync || *has_fetched.read() {
            return;
        }
        if let Some(albums) = albums_res.read().clone() {
            has_fetched.set(true);
            if albums.is_empty() {
                spawn(async move {
                    let _ = crate::server::subsonic_sync::sync_server_library(false).await;
                });
            }
        }
    });

    let pending_album_id = use_memo(move || {
        pending_album_id_for_playlist
            .read()
            .clone()
            .unwrap_or_default()
    });
    let pending_tracks_res = use_album_tracks(source, pending_album_id);

    rsx! {
        div {
            class: if cfg!(target_os = "android") { "px-4 pt-2 pb-28 absolute inset-0 flex flex-col" } else { "p-8 pb-24 absolute inset-0 flex flex-col" },

            if album_id.read().is_empty() {
                div {
                    if !cfg!(target_os = "android") {
                        h1 { class: "text-3xl font-bold text-white mb-6", "{i18n::t(\"all_albums\")}" }
                    }

                    AlbumGrid {
                        config,
                        album_id,
                        open_album_menu,
                        show_album_playlist_modal,
                        pending_album_id_for_playlist,
                    }

                    if *show_album_playlist_modal.read() {
                        PlaylistModal {
                            on_close: move |_| show_album_playlist_modal.set(false),
                            on_add_to_playlist: move |playlist_id: String| {
                                if pending_album_id_for_playlist.read().is_some() {
                                    let refs: Vec<String> = pending_tracks_res
                                        .read()
                                        .clone()
                                        .unwrap_or_default()
                                        .iter()
                                        .map(|t| t.id.key().into_owned())
                                        .collect();
                                    let s = active_source.peek().clone();
                                    spawn(async move {
                                        if !refs.is_empty()
                                            && s.add_to_playlist(&playlist_id, &refs).await.is_ok()
                                        {
                                            gens.bump(Table::Playlists);
                                        }
                                    });
                                }
                                show_album_playlist_modal.set(false);
                            },
                            on_create_playlist: move |name: String| {
                                if pending_album_id_for_playlist.read().is_some() {
                                    let refs: Vec<String> = pending_tracks_res
                                        .read()
                                        .clone()
                                        .unwrap_or_default()
                                        .iter()
                                        .map(|t| t.id.key().into_owned())
                                        .collect();
                                    let s = active_source.peek().clone();
                                    spawn(async move {
                                        if !refs.is_empty()
                                            && s.create_playlist(&name, &refs).await.is_ok()
                                        {
                                            gens.bump(Table::Playlists);
                                        }
                                    });
                                }
                                show_album_playlist_modal.set(false);
                            },
                        }
                    }
                }
            } else {
                AlbumDetail {
                    config,
                    album_id_str: album_id.read().clone(),
                    queue,
                    current_queue_index,
                    on_close: move |_| album_id.set(String::new()),
                }
            }
        }
    }
}

#[component]
fn AlbumGrid(
    config: Signal<AppConfig>,
    mut album_id: Signal<String>,
    mut open_album_menu: Signal<Option<String>>,
    mut show_album_playlist_modal: Signal<bool>,
    mut pending_album_id_for_playlist: Signal<Option<String>>,
) -> Element {
    let gens = hooks::db_reactivity::use_generations();
    let source = use_active_source();
    let active_source = use_context::<Signal<::server::source::ActiveSource>>();
    let caps = use_memo(move || active_source.read().capabilities());
    let is_offline = use_context::<Signal<bool>>();
    let mut ctrl = use_context::<hooks::use_player_controller::PlayerController>();
    let albums_res = use_albums(source);

    // Offline (server): only albums with downloaded tracks. Album ids come from
    // the downloaded tracks themselves. The grid dedupes by title — the detail
    // re-aggregates same-titled albums.
    let offline_keys = use_memo(move || -> Vec<String> {
        if !(caps().downloads && *is_offline.read()) {
            return Vec::new();
        }
        config
            .read()
            .offline_tracks
            .iter()
            .filter(|(_, p)| std::path::Path::new(p).exists())
            .map(|(id, _)| id.clone())
            .collect()
    });
    let offline_tracks_res = use_tracks_by_keys(source, offline_keys);
    let downloaded_album_ids = use_memo(move || -> HashSet<String> {
        if !(caps().downloads && *is_offline.read()) {
            return HashSet::new();
        }
        offline_tracks_res
            .read()
            .clone()
            .unwrap_or_default()
            .iter()
            .map(|t| t.album_id.clone())
            .collect()
    });

    let albums = use_memo(move || {
        let offline = caps().downloads && *is_offline.read();
        let downloaded = downloaded_album_ids();
        let mut albums = albums_res.read().clone().unwrap_or_default();
        albums.sort_by(|a, b| {
            a.title
                .trim()
                .to_lowercase()
                .cmp(&b.title.trim().to_lowercase())
        });
        let mut seen = HashSet::new();
        albums
            .into_iter()
            .filter(|a| !offline || downloaded.contains(&a.id))
            .filter(|a| seen.insert(a.title.trim().to_lowercase()))
            .collect::<Vec<_>>()
    });

    rsx! {
        div {
            if albums().is_empty() {
                p { class: "text-slate-500", "{i18n::t(\"no_albums_found\")}" }
            } else {
                div { class: "grid grid-cols-[repeat(auto-fill,minmax(180px,1fr))] gap-6",
                    for album in albums() {
                        {
                            let cap = caps();
                            let id_for_nav = album.id.clone();
                            let id_for_menu = album.id.clone();
                            let is_open = open_album_menu.read().as_deref() == Some(&album.id);
                            let cover_url = ::server::cover::from_path(&config.read(), album.cover_path.as_deref(), 360);
                            let remove_label = if cap.delete_from_disk {
                                i18n::t("delete_album").to_string()
                            } else {
                                i18n::t("remove_from_cache").to_string()
                            };
                            let actions = vec![
                                MenuAction::new(i18n::t("add_all_to_queue").as_str(), "fa-solid fa-list-ul"),
                                MenuAction::new(i18n::t("add_all_to_playlist").as_str(), "fa-solid fa-plus"),
                                MenuAction::new(remove_label.as_str(), "fa-solid fa-trash").destructive(),
                            ];
                            let tags = [AlbumAction::Queue, AlbumAction::Playlist, AlbumAction::Remove];
                            rsx! {
                                div {
                                    key: "{album.id}",
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
                                        onclick: move |_| album_id.set(id_for_nav.clone()),
                                        div {
                                            class: "aspect-square rounded-lg bg-stone-800 mb-3 overflow-hidden relative",
                                            style: "-webkit-user-drag: none;",
                                            ondragstart: move |evt| evt.prevent_default(),
                                            if let Some(url) = &cover_url {
                                                img { src: "{url}", class: "w-full h-full object-cover group-hover:scale-105 transition-transform duration-300", decoding: "async", loading: "lazy", draggable: "false", ondragstart: move |evt| evt.prevent_default() }
                                            } else {
                                                div { class: "w-full h-full flex items-center justify-center",
                                                    i { class: "fa-solid fa-compact-disc text-4xl text-white/20" }
                                                }
                                            }
                                        }
                                        h3 { class: "text-white font-medium truncate", "{album.title}" }
                                        p { class: "text-sm text-stone-400 truncate", "{album.artist}" }
                                    }

                                    div { class: "absolute bottom-3 right-3",
                                        DotsMenu {
                                            actions,
                                            is_open,
                                            on_open: {
                                                let id = id_for_menu.clone();
                                                move |_| open_album_menu.set(Some(id.clone()))
                                            },
                                            on_close: move |_| open_album_menu.set(None),
                                            button_class: "opacity-0 group-hover:opacity-100 focus:opacity-100 bg-black/40".to_string(),
                                            anchor: "right".to_string(),
                                            on_action: {
                                                let id = id_for_menu.clone();
                                                let title = album.title.clone();
                                                move |idx: usize| {
                                                    open_album_menu.set(None);
                                                    let Some(tag) = tags.get(idx).copied() else { return };
                                                    match tag {
                                                        AlbumAction::Queue => {
                                                            let s_src = source.peek().clone();
                                                            let read_db = consume_context::<hooks::ReadDb>();
                                                            let album_id = id.clone();
                                                            spawn(async move {
                                                                let mut tracks = read_db.album_tracks(&s_src, &album_id).await.unwrap_or_default();
                                                                tracks.sort_by(|a, b| {
                                                                    a.track_number.cmp(&b.track_number)
                                                                        .then_with(|| a.title.cmp(&b.title))
                                                                });
                                                                ctrl.add_to_queue(tracks);
                                                            });
                                                        }
                                                        AlbumAction::Playlist => {
                                                            pending_album_id_for_playlist.set(Some(id.clone()));
                                                            show_album_playlist_modal.set(true);
                                                        }
                                                        AlbumAction::Remove => {
                                                            if cap.delete_from_disk {
                                                                let s_src = source.peek().clone();
                                                                let read_db = consume_context::<hooks::ReadDb>();
                                                                let album_src = active_source.peek().clone();
                                                                let album_id = id.clone();
                                                                spawn(async move {
                                                                    let to_delete = read_db.album_tracks(&s_src, &album_id).await.unwrap_or_default();
                                                                    for track in &to_delete {
                                                                        if let Some(path) = track.id.local_path() {
                                                                            let _ = std::fs::remove_file(path);
                                                                        }
                                                                    }
                                                                    if album_src.delete_album(&album_id).await.is_ok() {
                                                                        gens.bump(Table::Tracks);
                                                                        gens.bump(Table::Albums);
                                                                    }
                                                                });
                                                            } else {
                                                                // Server: drop every same-titled album's cache.
                                                                let album_src = active_source.peek().clone();
                                                                let all = albums_res.read().clone().unwrap_or_default();
                                                                let ids: Vec<String> = all.iter().filter(|a| a.title == title).map(|a| a.id.clone()).collect();
                                                                spawn(async move {
                                                                    for aid in &ids {
                                                                        let _ = album_src.delete_album(aid).await;
                                                                    }
                                                                    gens.bump(Table::Tracks);
                                                                    gens.bump(Table::Albums);
                                                                });
                                                            }
                                                        }
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
fn AlbumDetail(
    config: Signal<AppConfig>,
    album_id_str: String,
    mut queue: Signal<Vec<reader::models::Track>>,
    current_queue_index: Signal<usize>,
    on_close: EventHandler<()>,
) -> Element {
    let gens = hooks::db_reactivity::use_generations();
    let nav_ctrl = use_context::<components::NavigationController>();
    let source = use_active_source();
    let active_source = use_context::<Signal<::server::source::ActiveSource>>();
    let caps = use_memo(move || active_source.read().capabilities());
    let is_offline = use_context::<Signal<bool>>();
    let download_queue = use_context::<Signal<DownloadQueue>>();

    let album_id_memo = use_memo(use_reactive!(|album_id_str| album_id_str));
    let album_res = use_album(source, album_id_memo);
    let albums_res = use_albums(source);

    let album_loading = album_res.read().is_none();
    let album = match album_res.read().clone().flatten() {
        Some(a) => a,
        None => {
            if album_loading {
                return rsx! { div {} };
            }
            return rsx! { div { "{i18n::t(\"album_not_found\")}" } };
        }
    };

    // The grid dedupes albums by title, so the detail aggregates every
    // same-titled album's tracks.
    let info_title = album.title.clone();
    let matching_ids = use_memo(move || -> Vec<String> {
        let title = info_title.clone();
        let ids: Vec<String> = albums_res
            .read()
            .clone()
            .unwrap_or_default()
            .into_iter()
            .filter(|a| a.title == title)
            .map(|a| a.id)
            .collect();
        ids
    });
    let tracks_res = {
        let read_db = use_context::<hooks::ReadDb>();
        use_resource(move || {
            let _ = gens.generation(Table::Tracks);
            let (read_db, s, ids) = (read_db.clone(), source(), matching_ids());
            async move {
                let mut out = Vec::new();
                for id in &ids {
                    out.extend(read_db.album_tracks(&s, id).await.unwrap_or_default());
                }
                out
            }
        })
    };

    let tracks = use_memo(move || {
        let offline = caps().downloads && *is_offline.read();
        let conf = config.read();
        let mut tracks: Vec<reader::models::Track> = tracks_res
            .read()
            .clone()
            .unwrap_or_default()
            .into_iter()
            .filter(|t| !offline || conf.offline_tracks.contains_key(t.id.key().as_ref()))
            .collect();
        tracks.sort_by(|a, b| {
            a.disc_number
                .unwrap_or(1)
                .cmp(&b.disc_number.unwrap_or(1))
                .then_with(|| {
                    a.track_number
                        .unwrap_or(0)
                        .cmp(&b.track_number.unwrap_or(0))
                })
        });
        tracks
    });

    let album_title = album.title.clone();
    let album_artist = album.artist.clone();
    let album_artist_for_nav = album_artist.clone();
    let cover_url = ::server::cover::from_path(&config.read(), album.cover_path.as_deref(), 512);
    let cap = caps();
    let aid = album.id.clone();

    let cover_cache = directories::ProjectDirs::from("com", "temidaradev", "kopuz")
        .map(|d| d.cache_dir().join("covers"))
        .unwrap_or_else(|| PathBuf::from("./cache/covers"));

    // Local-only custom cover reset.
    let cover_reset_action = if cap.edit_tags && album.cover_path.is_some() {
        let aid = aid.clone();
        let delete_cover = album.cover_path.clone();
        let cover_cache = cover_cache.clone();
        Some(rsx! {
            button {
                class: "inline-flex items-center justify-center h-9 w-9 rounded-full text-sm font-medium transition-colors border border-white/12 hover:bg-white/10",
                style: "color: var(--color-white); opacity: 0.6;",
                aria_label: i18n::t("remove_cover").to_string(),
                title: i18n::t("remove_cover").to_string(),
                onclick: move |_| {
                    let aid = aid.clone();
                    let delete_cover = delete_cover.clone();
                    let cover_cache = cover_cache.clone();
                    let local = consume_context::<Signal<::server::source::ActiveSource>>().peek().clone();
                    spawn(async move {
                        if local.update_album_cover(&aid, None, false).await.is_ok() {
                            gens.bump(Table::Albums);
                        }
                        if let Some(path) = delete_cover
                            && path.starts_with(&cover_cache)
                        {
                            let _ = tokio::fs::remove_file(&path).await;
                        }
                    });
                },
                i { class: "fa-solid fa-trash text-xs" }
            }
        })
    } else {
        None
    };

    let aid_cover = aid.clone();
    let tracks_delete = tracks();
    let tracks_download = tracks();
    let tracks_download_all = tracks();
    let tracks_delete_all = tracks();

    let is_downloading_all = cap.downloads && {
        let q = download_queue.read();
        tracks().iter().any(|t| {
            let key = t.id.key();
            q.items.iter().any(|i| {
                i.id.as_str() == key.as_ref()
                    && matches!(
                        i.status,
                        DownloadStatus::Queued | DownloadStatus::Downloading
                    )
            })
        })
    };

    rsx! {
        div { class: "absolute inset-0 flex flex-col overflow-hidden p-8",
            TrackListView {
                name: album_title,
                description: album_artist,
                on_description_click: Some(EventHandler::new(move |_| {
                    nav_ctrl.navigate_to_artist(album_artist_for_nav.clone());
                })),
                cover_url,
                is_album: true,
                back_label: i18n::t("back_to_albums").to_string(),
                tracks: tracks(),
                on_close,
                enable_metadata: cap.edit_tags,
                show_delete_in_selection: cap.delete_from_disk,
                is_downloading_all,
                on_cover_click: cap.edit_tags.then(|| EventHandler::new(move |_| {
                    let aid = aid_cover.clone();
                    let _ = &aid;
                    #[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
                    let local = consume_context::<Signal<::server::source::ActiveSource>>().peek().clone();
                    #[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
                    spawn(async move {
                        let file = rfd::AsyncFileDialog::new()
                            .add_filter("Images", &["jpg", "jpeg", "png", "webp"])
                            .pick_file()
                            .await;
                        if let Some(file) = file {
                            let path = file.path().to_path_buf();
                            let Ok(data) = tokio::fs::read(&path).await else { return };
                            let cover_cache = directories::ProjectDirs::from("com", "temidaradev", "kopuz")
                                .map(|d| d.cache_dir().join("covers"))
                                .unwrap_or_else(|| PathBuf::from("./cache/covers"));
                            if let Ok(saved) = reader::utils::save_cover(&aid, &data, path.extension().and_then(|e| e.to_str()), &cover_cache) {
                                let saved_str = saved.to_string_lossy().into_owned();
                                if local.update_album_cover(&aid, Some(&saved_str), true).await.is_ok() {
                                    gens.bump(Table::Albums);
                                }
                            }
                        }
                    });
                })),
                actions: cover_reset_action,
                on_delete_track: cap.delete_from_disk.then(|| EventHandler::new(move |idx: usize| {
                    if let Some(t) = tracks_delete.get(idx)
                        && let Some(track_path) = t.id.local_path()
                        && std::fs::remove_file(track_path).is_ok()
                    {
                        let s = consume_context::<Signal<::server::source::ActiveSource>>().peek().clone();
                        let key = t.id.key().into_owned();
                        spawn(async move {
                            if s.delete_tracks(&[key]).await.is_ok() {
                                gens.bump(Table::Tracks);
                            }
                        });
                    }
                })),
                on_selection_delete: cap.delete_from_disk.then(|| EventHandler::new(move |paths: Vec<PathBuf>| {
                    let mut keys = Vec::new();
                    for path in &paths {
                        if std::fs::remove_file(path).is_ok() {
                            keys.push(path.to_string_lossy().into_owned());
                        }
                    }
                    if !keys.is_empty() {
                        let s = consume_context::<Signal<::server::source::ActiveSource>>().peek().clone();
                        spawn(async move {
                            if s.delete_tracks(&keys).await.is_ok() {
                                gens.bump(Table::Tracks);
                            }
                        });
                    }
                })),
                on_download_track: cap.downloads.then(|| EventHandler::new(move |idx: usize| {
                    if let Some(t) = tracks_download.get(idx) {
                        let key = t.id.key();
                        if key.is_empty() {
                            return;
                        }
                        let key = key.as_ref();
                        let downloaded = config.read().offline_tracks.get(key)
                            .map(|p| std::path::Path::new(p).exists())
                            .unwrap_or(false);
                        if downloaded {
                            delete_downloads(vec![key.to_string()], config, download_queue);
                        } else {
                            queue_downloads(vec![(key.to_string(), t.title.clone(), t.artist.clone())], config, download_queue);
                        }
                    }
                })),
                on_download_all: cap.downloads.then(|| EventHandler::new(move |_: ()| {
                    let requests: Vec<(String, String, String)> = tracks_download_all.iter().filter_map(|t| {
                        let k = t.id.key();
                        (!k.is_empty()).then(|| (k.into_owned(), t.title.clone(), t.artist.clone()))
                    }).collect();
                    queue_downloads(requests, config, download_queue);
                })),
                on_delete_all: cap.downloads.then(|| EventHandler::new(move |_: ()| {
                    let ids: Vec<String> = tracks_delete_all.iter().filter_map(|t| {
                        let k = t.id.key();
                        (!k.is_empty()).then(|| k.into_owned())
                    }).collect();
                    delete_downloads(ids, config, download_queue);
                })),
            }
        }
    }
}
