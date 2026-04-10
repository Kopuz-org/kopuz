use components::playlist_modal::PlaylistModal;
use components::selection_bar::SelectionBar;
use config::{AppConfig, MusicService};
use dioxus::prelude::*;
use reader::{Library, PlaylistStore};
use ::server::jellyfin::JellyfinClient;
use ::server::subsonic::SubsonicClient;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

#[component]
pub fn JellyfinArtist(
    library: Signal<Library>,
    config: Signal<AppConfig>,
    artist_name: Signal<String>,
    playlist_store: Signal<PlaylistStore>,
    mut queue: Signal<Vec<reader::models::Track>>,
    mut current_queue_index: Signal<usize>,
) -> Element {
    let mut ctrl = use_context::<hooks::use_player_controller::PlayerController>();
    let mut show_playlist_modal = use_signal(|| false);
    let mut active_menu_track = use_signal(|| None::<PathBuf>);
    let mut selected_track_for_playlist = use_signal(|| None::<PathBuf>);

    // Multi-selection state
    let mut is_selection_mode = use_signal(|| false);
    let mut selected_tracks = use_signal(|| HashSet::<PathBuf>::new());

    let jellyfin_artists = use_memo(move || {
        let lib = library.read();
        let mut artist_map = HashMap::new();
        for album in &lib.jellyfin_albums {
            if !artist_map.contains_key(&album.artist) {
                artist_map.insert(album.artist.clone(), album.cover_path.clone());
            }
        }
        let mut artists: Vec<_> = artist_map.into_iter().collect();
        artists.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
        artists
    });

    let artist_tracks = use_memo(move || {
        let lib = library.read();
        let artist = artist_name.read();
        if artist.is_empty() {
            return Vec::new();
        }
        lib.jellyfin_tracks
            .iter()
            .filter(|t| t.artist.to_lowercase() == artist.to_lowercase())
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
        lib.jellyfin_albums
            .iter()
            .find(|a| a.artist.to_lowercase() == artist.to_lowercase())
            .and_then(|album| {
                if let Some(server) = &conf.server {
                    album.cover_path.as_ref().and_then(|cover_path| {
                        let path_str = cover_path.to_string_lossy();
                        utils::jellyfin_image::jellyfin_image_url_from_path(
                            &path_str,
                            &server.url,
                            server.access_token.as_deref(),
                            512,
                            90,
                        )
                    })
                } else {
                    None
                }
            })
    });

    let name = artist_name.read().clone();

    rsx! {
        div {
            if name.is_empty() {
                div { class: "grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 gap-8",
                    for (artist, cover_path) in jellyfin_artists() {
                        {
                            let cover_url = if let Some(server) = &config.read().server {
                                if let Some(path) = cover_path {
                                    let path_str = path.to_string_lossy();
                                    utils::jellyfin_image::jellyfin_image_url_from_path(
                                        &path_str,
                                        &server.url,
                                        server.access_token.as_deref(),
                                        320,
                                        80,
                                    )
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
                                    p { class: "text-xs text-slate-500 uppercase tracking-wider mt-1", "Artist" }
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
                                                                let _ =
                                                                    remote.add_to_playlist(&pid, item_id).await;
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

                    div { class: "flex items-center justify-between mb-8",
                        button {
                            class: "flex items-center gap-2 text-slate-400 hover:text-white transition-colors",
                            onclick: move |_| artist_name.set(String::new()),
                            i { class: "fa-solid fa-arrow-left" }
                            "Back to Artists"
                        }
                    }

                    components::showcase::Showcase {
                        name: name.clone(),
                        description: "Artist".to_string(),
                        cover_url: artist_cover(),
                        tracks: artist_tracks(),
                        library,
                        active_track: active_menu_track.read().clone(),
                        is_selection_mode: is_selection_mode(),
                        selected_tracks: selected_tracks.read().clone(),
                        on_long_press: move |idx: usize| {
                            if let Some(track) = artist_tracks().get(idx) {
                                is_selection_mode.set(true);
                                selected_tracks.write().insert(track.path.clone());
                            }
                        },
                        on_select: move |(idx, selected): (usize, bool)| {
                            if let Some(track) = artist_tracks().get(idx) {
                                if selected {
                                    selected_tracks.write().insert(track.path.clone());
                                } else {
                                    selected_tracks.write().remove(&track.path);
                                    if selected_tracks.read().is_empty() {
                                        is_selection_mode.set(false);
                                    }
                                }
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
                        on_delete_track: move |_| active_menu_track.set(None),
                        actions: None,
                    }
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
    queue: Signal<Vec<reader::models::Track>>,
    current_queue_index: Signal<usize>,
) -> Element {
    rsx! {
        JellyfinArtist {
            library,
            config,
            artist_name,
            playlist_store,
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
    queue: Signal<Vec<reader::models::Track>>,
    current_queue_index: Signal<usize>,
) -> Element {
    rsx! {
        JellyfinArtist {
            library,
            config,
            artist_name,
            playlist_store,
            queue,
            current_queue_index,
        }
    }
}
