use ::server::jellyfin::JellyfinClient;
use ::server::subsonic::SubsonicClient;
use components::playlist_modal::PlaylistModal;
use components::selection_bar::SelectionBar;
use components::track_row::TrackRow;
use config::{AppConfig, MusicService};
use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;
use reader::{FavoritesStore, Library, PlaylistStore};
use std::collections::HashSet;
use std::path::PathBuf;

#[component]
pub fn JellyfinFavorites(
    favorites_store: Signal<FavoritesStore>,
    library: Signal<Library>,
    config: Signal<AppConfig>,
    playlist_store: Signal<PlaylistStore>,
    mut queue: Signal<Vec<reader::models::Track>>,
) -> Element {
    let mut ctrl = use_context::<PlayerController>();
    let mut active_menu_track = use_signal(|| None::<PathBuf>);
    let mut has_synced = use_signal(|| false);
    let mut is_syncing = use_signal(|| false);

    // Multi-selection state
    let mut is_selection_mode = use_signal(|| false);
    let mut selected_tracks = use_signal(|| HashSet::<PathBuf>::new());
    let mut show_playlist_modal = use_signal(|| false);
    let mut selected_track_for_playlist = use_signal(|| None::<PathBuf>);

    use_effect(move || {
        if !*has_synced.read() {
            has_synced.set(true);
            is_syncing.set(true);
            spawn(async move {
                let (server_config, device_id) = {
                    let conf = config.peek();
                    if let Some(server) = &conf.server {
                        if let (Some(token), Some(user_id)) =
                            (&server.access_token, &server.user_id)
                        {
                            (
                                Some((
                                    server.service,
                                    server.url.clone(),
                                    token.clone(),
                                    user_id.clone(),
                                )),
                                conf.device_id.clone(),
                            )
                        } else {
                            (None, conf.device_id.clone())
                        }
                    } else {
                        (None, conf.device_id.clone())
                    }
                };

                let ids = if let Some((service, url, token, user_id)) = server_config {
                    match service {
                        MusicService::Jellyfin => {
                            let remote =
                                JellyfinClient::new(&url, Some(&token), &device_id, Some(&user_id));
                            remote
                                .get_favorite_items()
                                .await
                                .map(|items| items.into_iter().map(|i| i.id).collect())
                                .unwrap_or_default()
                        }
                        MusicService::Subsonic | MusicService::Custom => {
                            let remote = SubsonicClient::new(&url, &user_id, &token);
                            remote.get_starred_song_ids().await.unwrap_or_default()
                        }
                    }
                } else {
                    Vec::new()
                };

                let mut store = favorites_store.write();
                store.jellyfin_favorites = ids;
                is_syncing.set(false);
            });
        }
    });

    let displayed_tracks: Vec<(reader::models::Track, Option<String>)> = {
        let store = favorites_store.read();
        let lib = library.read();
        let server = config.read();
        let server_ref = server.server.as_ref().cloned();

        lib.jellyfin_tracks
            .iter()
            .filter(|t| {
                let path_str = t.path.to_string_lossy();
                let parts: Vec<&str> = path_str.split(':').collect();
                if parts.len() >= 2 {
                    store.is_jellyfin_favorite(parts[1])
                } else {
                    false
                }
            })
            .map(|t| {
                let cover_url = if let Some(ref srv) = server_ref {
                    let path_str = t.path.to_string_lossy();
                    utils::jellyfin_image::track_cover_url_with_album_fallback(
                        &path_str,
                        &t.album_id,
                        &srv.url,
                        srv.access_token.as_deref(),
                        80,
                        80,
                    )
                } else {
                    None
                };
                (t.clone(), cover_url)
            })
            .collect()
    };

    let queue_tracks: Vec<reader::models::Track> =
        displayed_tracks.iter().map(|(t, _)| t.clone()).collect();

    let is_empty = displayed_tracks.is_empty();

    let tracks_nodes = displayed_tracks
        .into_iter()
        .enumerate()
        .map(|(idx, (track, cover_url))| {
            let track_menu = track.clone();
            let track_path = track.path.clone();
            let track_select = track.path.clone();
            let track_add = track.clone();
            let queue_source = queue_tracks.clone();
            let track_key = format!("{}-{}", track.path.display(), idx);
            let is_menu_open = active_menu_track.read().as_ref() == Some(&track.path);
            let is_selected = selected_tracks.read().contains(&track_path);

            rsx! {
                TrackRow {
                    key: "{track_key}",
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
                    on_close_menu: move |_| active_menu_track.set(None),
                    on_delete: move |_| active_menu_track.set(None),
                    on_play: move |_| {
                        queue.set(queue_source.clone());
                        ctrl.play_track(idx);
                    },
                }
            }
        });

    rsx! {
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
                                    if let (Some(token), Some(user_id)) = (&server.access_token, &server.user_id) {
                                        for path in selected_paths {
                                            let parts: Vec<&str> = path.to_str().unwrap_or_default().split(':').collect();
                                            if parts.len() >= 2 {
                                                let item_id = parts[1];
                                                    match server.service {
                                                        MusicService::Jellyfin => {
                                                            let remote = JellyfinClient::new(
                                                                &server.url,
                                                                Some(token),
                                                                &conf.device_id,
                                                                Some(user_id),
                                                            );
                                                            let _ = remote.add_to_playlist(&pid, item_id).await;
                                                        }
                                                        MusicService::Subsonic | MusicService::Custom => {
                                                            let remote = SubsonicClient::new(&server.url, user_id, token);
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
                                    if let (Some(token), Some(user_id)) = (&server.access_token, &server.user_id) {
                                        let item_ids: Vec<String> = selected_paths
                                            .iter()
                                            .filter_map(|p| {
                                                let parts: Vec<&str> = p.to_str()?.split(':').collect();
                                                if parts.len() >= 2 {
                                                    Some(parts[1].to_string())
                                                } else {
                                                    None
                                                }
                                            })
                                            .collect();

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
                                                    let _ = remote.create_playlist(&playlist_name, &item_id_refs).await;
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

            if *is_syncing.read() {
                div {
                    class: "flex items-center gap-2 text-slate-400 text-sm mb-4",
                    i { class: "fa-solid fa-circle-notch fa-spin" }
                    span { "Syncing with server..." }
                }
            }

            if is_empty && !*is_syncing.read() {
                div {
                    class: "flex flex-col items-center justify-center h-64 text-slate-500",
                    i { class: "fa-regular fa-heart text-4xl mb-4 opacity-30" }
                    p { class: "text-base", "No favorites yet." }
                    p { class: "text-sm mt-1 opacity-70",
                        "Heart a track while it's playing to add it here and sync it to your server."
                    }
                }
            } else if !is_empty {
                div {
                    class: "space-y-1",
                    {tracks_nodes}
                }
            }
        }
    }
}

pub use JellyfinFavorites as ServerFavorites;
