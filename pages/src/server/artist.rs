use crate::server::download_manager::{DownloadQueue, queue_downloads};
use components::dots_menu::{DotsMenu, MenuAction};
use components::playlist_modal::PlaylistModal;
use components::selection_bar::SelectionBar;
use config::{AppConfig, ArtistViewOrder, MusicService};
use dioxus::prelude::*;
use reader::{Library, PlaylistStore};
use server::jellyfin::JellyfinClient;
use server::subsonic::SubsonicClient;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

#[component]
pub fn JellyfinArtist(
    library: Signal<Library>,
    config: Signal<AppConfig>,
    artist_name: Signal<String>,
    playlist_store: Signal<PlaylistStore>,
    on_navigate: EventHandler<String>,
    mut queue: Signal<Vec<reader::models::Track>>,
    mut current_queue_index: Signal<usize>,
) -> Element {
    let is_offline = use_context::<Signal<bool>>();
    let mut ctrl = use_context::<hooks::use_player_controller::PlayerController>();
    let mut show_playlist_modal = use_signal(|| false);
    let mut active_menu_track = use_signal(|| None::<PathBuf>);
    let mut selected_track_for_playlist = use_signal(|| None::<PathBuf>);

    let mut is_selection_mode = use_signal(|| false);
    let mut selected_tracks = use_signal(|| HashSet::<PathBuf>::new());
    let download_queue = use_context::<Signal<DownloadQueue>>();

    let sort_order = use_signal(move || config.read().artist_view_order.clone());
    use_effect(move || {
        let curr = sort_order.read().clone();
        if config.peek().artist_view_order != curr {
            config.write().artist_view_order = curr;
        }
    });

    let mut open_album_menu = use_signal(|| None::<String>);
    let mut show_album_playlist_modal = use_signal(|| false);
    let mut pending_album_id_for_playlist = use_signal(|| None::<String>);

    let jellyfin_artists = use_memo(move || {
        let lib = library.read();
        let mut artist_map: HashMap<String, Option<PathBuf>> = HashMap::new();
        for album in &lib.jellyfin_albums {
            if !artist_map.contains_key(&album.artist) {
                artist_map.insert(album.artist.clone(), album.cover_path.clone());
            }
        }
        for name in artist_map.keys().cloned().collect::<Vec<_>>() {
            if let Some(url) = lib.server_artist_images.get(&name) {
                artist_map.insert(name, Some(PathBuf::from(format!("directurl:{}", url))));
            }
        }
        let offline = *is_offline.read();
        let conf = config.read();
        let mut artists: Vec<_> = artist_map
            .into_iter()
            .filter(|(artist, _)| {
                if !offline {
                    return true;
                }
                lib.jellyfin_tracks.iter().any(|t| {
                    if t.artist.to_lowercase() != artist.to_lowercase() {
                        return false;
                    }
                    let s = t.path.to_string_lossy();
                    let id = s.split(':').nth(1).unwrap_or(&s);
                    if let Some(path_str) = conf.offline_tracks.get(id) {
                        std::path::Path::new(path_str).exists()
                    } else {
                        false
                    }
                })
            })
            .collect();
        artists.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
        artists
    });

    let artist_tracks = use_memo(move || {
        let lib = library.read();
        let artist = artist_name.read();
        if artist.is_empty() {
            return Vec::new();
        }
        let offline = *is_offline.read();
        let conf = config.read();
        lib.jellyfin_tracks
            .iter()
            .filter(|t| t.artist.to_lowercase() == artist.to_lowercase())
            .filter(|t| {
                if !offline {
                    return true;
                }
                let s = t.path.to_string_lossy();
                let id = s.split(':').nth(1).unwrap_or(&s);
                if let Some(path_str) = conf.offline_tracks.get(id) {
                    std::path::Path::new(path_str).exists()
                } else {
                    false
                }
            })
            .cloned()
            .collect::<Vec<_>>()
    });

    let artist_cover = use_memo(move || {
        let lib = library.read();
        let conf = config.read();
        let artist = artist_name.read();
        if artist.is_empty() {
            return None;
        }
        if let Some(url) = lib.server_artist_images.get(artist.as_str()) {
            return Some(utils::cover_url_from_string(url.clone()));
        }
        lib.jellyfin_albums
            .iter()
            .find(|a| a.artist.to_lowercase() == artist.to_lowercase())
            .and_then(|album| {
                if let Some(server) = &conf.server {
                    utils::map_cover_url(album.cover_path.as_ref().and_then(|cover_path| {
                        let path_str = cover_path.to_string_lossy();
                        utils::jellyfin_image::jellyfin_image_url_from_path(
                            &path_str,
                            &server.url,
                            server.access_token.as_deref(),
                            512,
                            90,
                        )
                    }))
                } else {
                    None
                }
            })
    });

    let artist_albums = use_memo(move || {
        let lib = library.read();
        let artist = artist_name.read();
        if artist.is_empty() {
            return Vec::new();
        }
        let artist_lc = artist.to_lowercase();
        let offline = *is_offline.read();
        let conf = config.read();
        let mut albums: Vec<_> = lib
            .jellyfin_albums
            .iter()
            .filter(|a| a.artist.to_lowercase() == artist_lc)
            .filter(|a| {
                if !offline {
                    return true;
                }
                lib.jellyfin_tracks.iter().any(|t| {
                    if t.album_id != a.id {
                        return false;
                    }
                    let s = t.path.to_string_lossy();
                    let id = s.split(':').nth(1).unwrap_or(&s);
                    if let Some(path_str) = conf.offline_tracks.get(id) {
                        std::path::Path::new(path_str).exists()
                    } else {
                        false
                    }
                })
            })
            .cloned()
            .collect();
        albums.sort_by(|a, b| {
            a.title
                .trim()
                .to_lowercase()
                .cmp(&b.title.trim().to_lowercase())
        });
        let mut seen = HashSet::new();
        albums.retain(|album| seen.insert(album.title.trim().to_lowercase()));
        albums
    });

    let name = artist_name.read().clone();

    let tracks_for_album = |library: &Library, album_id: &str| -> Vec<PathBuf> {
        library
            .jellyfin_tracks
            .iter()
            .filter(|t| t.album_id == album_id)
            .map(|t| t.path.clone())
            .collect()
    };

    rsx! {
        div {
            if name.is_empty() {
                div { class: "grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 gap-8",
                    for (artist, cover_path) in jellyfin_artists() {
                        {
                            let cover_url = if let Some(server) = &config.read().server {
                                if let Some(path) = cover_path {
                                    let path_str = path.to_string_lossy();
                                    utils::map_cover_url(utils::jellyfin_image::jellyfin_image_url_from_path(
                                        &path_str,
                                        &server.url,
                                        server.access_token.as_deref(),
                                        320,
                                        80,
                                    ))
                                } else {
                                    None
                                }
                            } else {
                                None
                            };

                            let art = artist.clone();
                            rsx! {
                                div {
                                    key: "{artist}",
                                    class: "group cursor-pointer flex flex-col items-center",
                                    style: "content-visibility: auto; contain-intrinsic-size: 0 180px;",
                                    onclick: move |_| artist_name.set(art.clone()),
                                    div { class: "aspect-square w-full rounded-full bg-stone-800 mb-4 overflow-hidden relative transition-all",
                                        if let Some(url) = cover_url {
                                            img {
                                                src: "{url}",
                                                class: "w-full h-full object-cover",
                                                decoding: "async", loading: "lazy"
                                            }
                                        } else {
                                            div { class: "w-full h-full flex items-center justify-center text-white/20",
                                                i { class: "fa-solid fa-microphone text-5xl" }
                                            }
                                        }
                                    }
                                    h3 { class: "text-white font-medium truncate text-center w-full group-hover:text-indigo-400 transition-colors", "{artist}" }
                                    p { class: "text-xs text-slate-500 uppercase tracking-wider mt-1", "{i18n::t(\"artist\")}" }
                                }
                            }
                        }
                    }
                }
            } else {
                div {
                    if *show_playlist_modal.read() {
                        PlaylistModal {
                            playlist_store,
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
                                        let conf = config.peek();
                                        if let Some(server) = &conf.server {
                                            if let (Some(token), Some(user_id)) =
                                                (&server.access_token, &server.user_id)
                                            {
                                                match server.service {
                                                    MusicService::Jellyfin => {
                                                        let remote = JellyfinClient::new(
                                                            &server.url,
                                                            Some(token),
                                                            &conf.device_id,
                                                            Some(user_id),
                                                        );
                                                        for path in selected_paths {
                                                            let parts: Vec<&str> = path
                                                                .to_str()
                                                                .unwrap_or_default()
                                                                .split(':')
                                                                .collect();
                                                            if parts.len() >= 2 {
                                                                let item_id = parts[1];
                                                                let _ = remote.add_to_playlist(&pid, item_id).await;
                                                            }
                                                        }
                                                    }
                                                    MusicService::Subsonic | MusicService::Custom => {
                                                        let remote = SubsonicClient::new(&server.url, user_id, token);
                                                        for path in selected_paths {
                                                            let parts: Vec<&str> = path
                                                                .to_str()
                                                                .unwrap_or_default()
                                                                .split(':')
                                                                .collect();
                                                            if parts.len() >= 2 {
                                                                let item_id = parts[1];
                                                                let _ = remote.add_to_playlist(&pid, item_id).await;
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
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
                                        let conf = config.peek();
                                        if let Some(server) = &conf.server {
                                            if let (Some(token), Some(user_id)) =
                                                (&server.access_token, &server.user_id)
                                            {
                                                let item_ids: Vec<String> = selected_paths.iter().filter_map(|p| {
                                                    let parts: Vec<&str> = p.to_str()?.split(':').collect();
                                                    if parts.len() >= 2 { Some(parts[1].to_string()) } else { None }
                                                }).collect();
                                                if !item_ids.is_empty() {
                                                    let item_id_refs: Vec<&str> = item_ids.iter().map(|s| s.as_str()).collect();
                                                    match server.service {
                                                        MusicService::Jellyfin => {
                                                            let remote = JellyfinClient::new(
                                                                &server.url,
                                                                Some(token),
                                                                &conf.device_id,
                                                                Some(user_id),
                                                            );
                                                            let _ = remote
                                                                .create_playlist(&playlist_name, &item_id_refs)
                                                                .await;
                                                        }
                                                        MusicService::Subsonic | MusicService::Custom => {
                                                            let remote = SubsonicClient::new(&server.url, user_id, token);
                                                            let _ = remote
                                                                .create_playlist(&playlist_name, &item_id_refs)
                                                                .await;
                                                        }
                                                    }
                                                }
                                            }
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

                    if *sort_order.read() == ArtistViewOrder::Albums {
                        if *show_album_playlist_modal.read() {
                            PlaylistModal {
                                playlist_store,
                                is_jellyfin: true,
                                on_close: move |_| show_album_playlist_modal.set(false),
                                on_add_to_playlist: move |playlist_id: String| {
                                    if let Some(album_id) = pending_album_id_for_playlist.read().clone() {
                                        let lib = library.read();
                                        let paths = tracks_for_album(&lib, &album_id);
                                        drop(lib);
                                        let pid = playlist_id.clone();
                                        spawn(async move {
                                            let conf = config.peek();
                                            if let Some(server) = &conf.server {
                                                if let (Some(token), Some(user_id)) =
                                                    (&server.access_token, &server.user_id)
                                                {
                                                    match server.service {
                                                        MusicService::Jellyfin => {
                                                            let remote = JellyfinClient::new(
                                                                &server.url,
                                                                Some(token),
                                                                &conf.device_id,
                                                                Some(user_id),
                                                            );
                                                            for path in paths {
                                                                let parts: Vec<&str> = path
                                                                    .to_str()
                                                                    .unwrap_or_default()
                                                                    .split(':')
                                                                    .collect();
                                                                if parts.len() >= 2 {
                                                                    let item_id = parts[1];
                                                                    let _ = remote.add_to_playlist(&pid, item_id).await;
                                                                }
                                                            }
                                                        }
                                                        MusicService::Subsonic | MusicService::Custom => {
                                                            let remote = SubsonicClient::new(&server.url, user_id, token);
                                                            for path in paths {
                                                                let parts: Vec<&str> = path
                                                                    .to_str()
                                                                    .unwrap_or_default()
                                                                    .split(':')
                                                                    .collect();
                                                                if parts.len() >= 2 {
                                                                    let item_id = parts[1];
                                                                    let _ = remote.add_to_playlist(&pid, item_id).await;
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        });
                                    }
                                    show_album_playlist_modal.set(false);
                                    pending_album_id_for_playlist.set(None);
                                },
                                on_create_playlist: move |playlist_name: String| {
                                    let paths = pending_album_id_for_playlist
                                        .read()
                                        .as_deref()
                                        .map(|id| {
                                            let lib = library.read();
                                            tracks_for_album(&lib, id)
                                        })
                                        .unwrap_or_default();
                                    spawn(async move {
                                        let conf = config.peek();
                                        if let Some(server) = &conf.server {
                                            if let (Some(token), Some(user_id)) =
                                                (&server.access_token, &server.user_id)
                                            {
                                                let item_ids: Vec<String> = paths.iter().filter_map(|p| {
                                                    let parts: Vec<&str> = p.to_str()?.split(':').collect();
                                                    if parts.len() >= 2 { Some(parts[1].to_string()) } else { None }
                                                }).collect();
                                                if !item_ids.is_empty() {
                                                    let item_id_refs: Vec<&str> = item_ids.iter().map(|s| s.as_str()).collect();
                                                    match server.service {
                                                        MusicService::Jellyfin => {
                                                            let remote = JellyfinClient::new(
                                                                &server.url,
                                                                Some(token),
                                                                &conf.device_id,
                                                                Some(user_id),
                                                            );
                                                            let _ = remote
                                                                .create_playlist(&playlist_name, &item_id_refs)
                                                                .await;
                                                        }
                                                        MusicService::Subsonic | MusicService::Custom => {
                                                            let remote = SubsonicClient::new(&server.url, user_id, token);
                                                            let _ = remote
                                                                .create_playlist(&playlist_name, &item_id_refs)
                                                                .await;
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    });
                                    show_album_playlist_modal.set(false);
                                    pending_album_id_for_playlist.set(None);
                                },
                            }
                        }

                        SortOrderToggle { sort_order }

                        if artist_albums().is_empty() {
                            p { class: "text-slate-500", "{i18n::t(\"no_albums_found\")}" }
                        } else {
                            {
                                let add_all_to_playlist_text = i18n::t("add_all_to_playlist").to_string();
                                let download_album_text = "Download Album".to_string();
                                rsx! {
                                    div { class: "grid grid-cols-[repeat(auto-fill,minmax(180px,1fr))] gap-6",
                                        for album in artist_albums() {
                                            {
                                                let id_for_menu = album.id.clone();
                                                let id_for_action = album.id.clone();
                                                let id_for_navigate = album.id.clone();
                                                let is_open = open_album_menu.read().as_deref() == Some(&album.id);
                                                let is_downloaded = {
                                                    let lib = library.peek();
                                                    let conf = config.read();
                                                    let aid = album.id.clone();
                                                    let tracks: Vec<_> = lib.jellyfin_tracks.iter().filter(|t| t.album_id == aid).collect();
                                                    !tracks.is_empty() && tracks.iter().all(|t| {
                                                        let s = t.path.to_string_lossy();
                                                        let tid = s.split(':').nth(1).unwrap_or(&s);
                                                        if let Some(path_str) = conf.offline_tracks.get(tid) {
                                                            std::path::Path::new(path_str).exists()
                                                        } else {
                                                            false
                                                        }
                                                    })
                                                };
                                                let album_menu_actions = vec![
                                                    MenuAction::new(add_all_to_playlist_text.as_str(), "fa-solid fa-list-music"),
                                                    if is_downloaded {
                                                        MenuAction::new("Remove downloads", "fa-solid fa-trash").destructive()
                                                    } else {
                                                        MenuAction::new(download_album_text.as_str(), "fa-solid fa-download")
                                                    }
                                                ];
                                                let cover_url = if let Some(server) = &config.read().server {
                                    utils::map_cover_url(album.cover_path.as_ref().and_then(|p| {
                                        let path_str = p.to_string_lossy();
                                        utils::jellyfin_image::jellyfin_image_url_from_path(
                                            &path_str,
                                            &server.url,
                                            server.access_token.as_deref(),
                                            320,
                                            80,
                                        )
                                    }))
                                } else {
                                    None
                                };
                                                rsx! {
                                                    div {
                                                        key: "{album.id}",
                                                        class: "group relative p-4 bg-white/5 rounded-xl hover:bg-white/10 transition-colors",
                                                        style: "content-visibility: auto; contain-intrinsic-size: 0 230px;",
                                                        onclick: move |_| on_navigate.call(id_for_navigate.clone()),
                                                        oncontextmenu: {
                                                            let id = id_for_menu.clone();
                                                            move |evt| {
                                                                evt.prevent_default();
                                                                open_album_menu.set(Some(id.clone()));
                                                            }
                                                        },
                                                        div { class: "cursor-pointer",
                                                            div { class: "aspect-square rounded-lg bg-stone-800 mb-3 overflow-hidden relative",
                                                                if let Some(url) = &cover_url {
                                                                    img {
                                                                        src: "{url}",
                                                                        class: "w-full h-full object-cover group-hover:scale-105 transition-transform duration-300",
                                                                        decoding: "async", loading: "lazy",
                                                                    }
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
                                                                        if idx == 0 {
                                                                            pending_album_id_for_playlist.set(Some(id.clone()));
                                                                            show_album_playlist_modal.set(true);
                                                                        } else if idx == 1 {
                                                                            if is_downloaded {
                                                                                let album_id = id.clone();
                                                                                let ids: Vec<String> = {
                                                                                    let lib = library.read();
                                                                                    lib.jellyfin_tracks
                                                                                        .iter()
                                                                                        .filter(|t| t.album_id == album_id)
                                                                                        .filter_map(|t| {
                                                                                            let s = t.path.to_string_lossy().to_string();
                                                                                            s.split(':').nth(1).map(|id| id.to_string())
                                                                                        })
                                                                                        .collect()
                                                                                };
                                                                                crate::server::download_manager::delete_downloads(ids, config, download_queue);
                                                                            } else {
                                                                                let album_id = id.clone();
                                                                                let requests: Vec<(String, String, String)> = {
                                                                                    let lib = library.read();
                                                                                    lib.jellyfin_tracks
                                                                                        .iter()
                                                                                        .filter(|t| t.album_id == album_id)
                                                                                        .filter_map(|t| {
                                                                                            let s = t.path.to_string_lossy().to_string();
                                                                                            s.split(':').nth(1).map(|id| (id.to_string(), t.title.clone(), t.artist.clone()))
                                                                                        })
                                                                                        .collect()
                                                                                };
                                                                                queue_downloads(requests, config, download_queue);
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
                    } else {
                        if artist_tracks().is_empty() {
                            div {
                                class: "flex flex-col items-center justify-center h-64 text-slate-500",
                                i { class: "fa-regular fa-music text-4xl mb-4 opacity-30" }
                                p { class: "text-base", "{i18n::t(\"no_tracks_found\")}" }
                            }
                        } else {
                            components::showcase::Showcase {
                                name: name.clone(),
                                description: i18n::t("artist").to_string(),
                                cover_url: artist_cover(),
                                tracks: artist_tracks(),
                                library,
                                active_track: active_menu_track.read().clone(),
                                is_selection_mode: is_selection_mode(),
                                selected_tracks: selected_tracks.read().clone(),
                                all_selected: !artist_tracks().is_empty() && artist_tracks().iter().all(|track| selected_tracks.read().contains(&track.path)),
                                on_select_all: move |selected: bool| {
                                    if selected {
                                        selected_tracks.set(artist_tracks().into_iter().map(|track| track.path).collect());
                                        is_selection_mode.set(true);
                                    } else {
                                        selected_tracks.write().clear();
                                        is_selection_mode.set(false);
                                    }
                                },
                                on_long_press: move |idx: usize| {
                                    if let Some(track) = artist_tracks().get(idx) {
                                        is_selection_mode.set(true);
                                        selected_tracks.write().insert(track.path.clone());
                                    }
                                },
                                on_select: move |(idx, selected): (usize, bool)| {
                                    if let Some(track) = artist_tracks().get(idx) {
                                        if selected {
                                            is_selection_mode.set(true);
                                            selected_tracks.write().insert(track.path.clone());
                                        } else {
                                            selected_tracks.write().remove(&track.path);
                                            if selected_tracks.read().is_empty() {
                                                is_selection_mode.set(false);
                                            }
                                        }
                                    }
                                },
                                on_play_all: move |_| {
                                      let is_shuffle = *ctrl.shuffle.peek();
                                      if is_shuffle {
                                          ctrl.play_queue_shuffled(artist_tracks());
                                      } else {
                                          ctrl.play_queue_linear(artist_tracks());
                                      }
                                  },
                                on_play: move |idx: usize| {
                                    let tracks = artist_tracks();
                                    queue.set(tracks.clone());
                                    current_queue_index.set(idx);
                                    ctrl.play_track(idx);
                                },
                                on_click_menu: move |idx: usize| {
                                    if let Some(track) = artist_tracks().get(idx) {
                                        if active_menu_track.read().as_ref() == Some(&track.path) {
                                            active_menu_track.set(None);
                                        } else {
                                            active_menu_track.set(Some(track.path.clone()));
                                        }
                                    }
                                },
                                on_close_menu: move |_| active_menu_track.set(None),
                                on_add_to_playlist: move |idx: usize| {
                                    if let Some(track) = artist_tracks().get(idx) {
                                        selected_track_for_playlist.set(Some(track.path.clone()));
                                        show_playlist_modal.set(true);
                                        active_menu_track.set(None);
                                    }
                                },
                                on_queue: move |idx: usize| {
                                    if let Some(track) = artist_tracks().get(idx) {
                                        ctrl.add_to_queue(vec![track.clone()]);
                                        active_menu_track.set(None);
                                    }
                                },
                                on_delete_track: move |_| active_menu_track.set(None),
                                on_download_track: move |idx: usize| {
                                    if let Some(track) = artist_tracks().get(idx) {
                                        let s = track.path.to_string_lossy();
                                        if let Some(item_id) = s.split(':').nth(1) {
                                            let is_downloaded = if let Some(path_str) = config.read().offline_tracks.get(item_id) {
                                                std::path::Path::new(path_str).exists()
                                            } else {
                                                false
                                            };
                                            if is_downloaded {
                                                crate::server::download_manager::delete_downloads(
                                                    vec![item_id.to_string()],
                                                    config,
                                                    download_queue,
                                                );
                                            } else {
                                                queue_downloads(
                                                    vec![(item_id.to_string(), track.title.clone(), track.artist.clone())],
                                                    config,
                                                    download_queue,
                                                );
                                            }
                                        }
                                        active_menu_track.set(None);
                                    }
                                },
                                on_download_all: move |_| {
                                    let requests: Vec<(String, String, String)> = artist_tracks()
                                        .iter()
                                        .filter_map(|t| {
                                            let s = t.path.to_string_lossy().to_string();
                                            s.split(':').nth(1).map(|id| (id.to_string(), t.title.clone(), t.artist.clone()))
                                        })
                                        .collect();
                                    queue_downloads(requests, config, download_queue);
                                },
                                on_delete_all: move |_| {
                                    let ids: Vec<String> = artist_tracks()
                                        .iter()
                                        .filter_map(|t| {
                                            let s = t.path.to_string_lossy().to_string();
                                            s.split(':').nth(1).map(|id| id.to_string())
                                        })
                                        .collect();
                                    crate::server::download_manager::delete_downloads(ids, config, download_queue);
                                },
                                is_downloading_all: download_queue.read().is_active(),
                                actions: Some(rsx! {
                                    SortOrderToggle { sort_order }
                                }),
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn SortOrderToggle(mut sort_order: Signal<ArtistViewOrder>) -> Element {
    let is_tracks = *sort_order.read() == ArtistViewOrder::Tracks;

    let btn_active =
        "px-3 py-1 text-xs rounded-md bg-white/10 text-white font-medium transition-all";
    let btn_inactive =
        "px-3 py-1 text-xs rounded-md text-white/40 hover:text-white/80 transition-all";

    rsx! {
        div { class: "flex justify-end mb-4",
            div { class: "flex space-x-1 bg-white/5 border border-white/5 p-1 rounded-lg",
                button {
                    class: if is_tracks { btn_active } else { btn_inactive },
                    onclick: move |_| sort_order.set(ArtistViewOrder::Tracks),
                    "{i18n::t(\"tracks\")}"
                }
                button {
                    class: if !is_tracks { btn_active } else { btn_inactive },
                    onclick: move |_| sort_order.set(ArtistViewOrder::Albums),
                    "{i18n::t(\"albums\")}"
                }
            }
        }
    }
}

#[component]
pub fn ServerArtist(
    library: Signal<Library>,
    config: Signal<AppConfig>,
    artist_name: Signal<String>,
    playlist_store: Signal<PlaylistStore>,
    on_navigate: EventHandler<String>,
    queue: Signal<Vec<reader::models::Track>>,
    current_queue_index: Signal<usize>,
) -> Element {
    let service = config
        .read()
        .active_service()
        .unwrap_or(MusicService::Jellyfin);

    match service {
        MusicService::Jellyfin => rsx! {
            JellyfinArtist {
                library,
                config,
                artist_name,
                playlist_store,
                on_navigate,
                queue,
                current_queue_index,
            }
        },
        MusicService::Subsonic => rsx! {
            SubsonicArtist {
                library,
                config,
                artist_name,
                playlist_store,
                on_navigate,
                queue,
                current_queue_index,
            }
        },
        MusicService::Custom => rsx! {
            CustomArtist {
                library,
                config,
                artist_name,
                playlist_store,
                on_navigate,
                queue,
                current_queue_index,
            }
        },
    }
}

#[component]
pub fn SubsonicArtist(
    library: Signal<Library>,
    config: Signal<AppConfig>,
    artist_name: Signal<String>,
    playlist_store: Signal<PlaylistStore>,
    on_navigate: EventHandler<String>,
    queue: Signal<Vec<reader::models::Track>>,
    current_queue_index: Signal<usize>,
) -> Element {
    rsx! {
        JellyfinArtist {
            library,
            config,
            artist_name,
            playlist_store,
            on_navigate,
            queue,
            current_queue_index,
        }
    }
}

#[component]
pub fn CustomArtist(
    library: Signal<Library>,
    config: Signal<AppConfig>,
    artist_name: Signal<String>,
    playlist_store: Signal<PlaylistStore>,
    on_navigate: EventHandler<String>,
    queue: Signal<Vec<reader::models::Track>>,
    current_queue_index: Signal<usize>,
) -> Element {
    rsx! {
        JellyfinArtist {
            library,
            config,
            artist_name,
            playlist_store,
            on_navigate,
            queue,
            current_queue_index,
        }
    }
}
