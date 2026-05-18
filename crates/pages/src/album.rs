use config::{AppConfig, MusicService, MusicSource};
use dioxus::prelude::*;
use reader::{Library, PlaylistStore};
use ::server::jellyfin::JellyfinClient;
use ::server::subsonic::SubsonicClient;

use crate::local::album::LocalAlbum;
use crate::server::album::{ServerAlbum, ServerAlbumDetails};

#[component]
pub fn Album(
    library: Signal<Library>,
    config: Signal<AppConfig>,
    album_id: Signal<String>,
    playlist_store: Signal<PlaylistStore>,
    mut queue: Signal<Vec<reader::models::Track>>,
    mut current_queue_index: Signal<usize>,
) -> Element {
    let is_server = config.read().active_source == MusicSource::Server;

    let open_album_menu = use_signal(|| None::<String>);
    let mut show_album_playlist_modal = use_signal(|| false);
    let pending_album_id_for_playlist = use_signal(|| None::<String>);

    let mut has_fetched_jellyfin = use_signal(|| false);

    let mut fetch_jellyfin = move || {
        has_fetched_jellyfin.set(true);
        spawn(async move {
            let _ = crate::server::subsonic_sync::sync_server_library(library, config, false).await;
        });
    };

    use_effect(move || {
        if is_server && !*has_fetched_jellyfin.read() {
            if library.read().jellyfin_tracks.is_empty()
                || library.read().jellyfin_albums.is_empty()
            {
                fetch_jellyfin();
            } else {
                has_fetched_jellyfin.set(true);
            }
        }
    });

    rsx! {
        div {
            class: "p-8 pb-24",

            if album_id.read().is_empty() {
                div {
                    h1 { class: "text-3xl font-bold text-white mb-6", "{i18n::t(\"all_albums\")}" }

                    if is_server {
                        ServerAlbum {
                            library,
                            config,
                            album_id,
                            playlist_store,
                            queue,
                            open_album_menu,
                            show_album_playlist_modal,
                            pending_album_id_for_playlist,
                        }
                    } else {
                        LocalAlbum {
                            library,
                            album_id,
                            playlist_store,
                            queue,
                            open_album_menu,
                            show_album_playlist_modal,
                            pending_album_id_for_playlist,
                        }
                    }

                    if *show_album_playlist_modal.read() {
                        components::playlist_modal::PlaylistModal {
                            playlist_store,
                            is_jellyfin: is_server,
                            on_close: move |_| show_album_playlist_modal.set(false),
                            on_add_to_playlist: move |playlist_id: String| {
                                if let Some(aid) = pending_album_id_for_playlist.read().clone() {
                                    let tracks: Vec<_> = {
                                        let lib = library.read();
                                        if is_server {
                                            lib.jellyfin_tracks.iter()
                                                .filter(|t| t.album_id == aid)
                                                .map(|t| t.path.clone())
                                                .collect()
                                        } else {
                                            let album_title = lib.albums.iter()
                                                .find(|a| a.id == aid)
                                                .map(|a| a.title.clone());
                                            if let Some(title) = album_title {
                                                lib.tracks.iter()
                                                    .filter(|t| t.album == title)
                                                    .map(|t| t.path.clone())
                                                    .collect()
                                            } else {
                                                Vec::new()
                                            }
                                        }
                                    };
                                    if is_server {
                                        let pid = playlist_id.clone();
                                        let paths = tracks.clone();
                                        let server_vals = {
                                            let conf = config.peek();
                                            conf.server.as_ref().and_then(|s| {
                                                if let (Some(tok), Some(uid)) = (&s.access_token, &s.user_id) {
                                                    Some((s.service, s.url.clone(), tok.clone(), uid.clone(), conf.device_id.clone()))
                                                } else { None }
                                            })
                                        };
                                        if let Some((service, url, token, user_id, device_id)) = server_vals {
                                            spawn(async move {
                                                let item_ids: Vec<String> = paths.iter()
                                                    .filter_map(|p| {
                                                        let parts: Vec<&str> = p.to_str()?.split(':').collect();
                                                        if parts.len() >= 2 { Some(parts[1].to_string()) } else { None }
                                                    })
                                                    .collect();
                                                let mut added = Vec::new();
                                                match service {
                                                    MusicService::Jellyfin => {
                                                        let remote = JellyfinClient::new(&url, Some(&token), &device_id, Some(&user_id));
                                                        for id in &item_ids {
                                                            if remote.add_to_playlist(&pid, id).await.is_ok() {
                                                                added.push(id.clone());
                                                            }
                                                        }
                                                    }
                                                    MusicService::Subsonic | MusicService::Custom => {
                                                        let remote = SubsonicClient::new(&url, &user_id, &token);
                                                        for id in &item_ids {
                                                            if remote.add_to_playlist(&pid, id).await.is_ok() {
                                                                added.push(id.clone());
                                                            }
                                                        }
                                                    }
                                                }
                                                if !added.is_empty() {
                                                    let mut store = playlist_store.write();
                                                    if let Some(pl) = store.jellyfin_playlists.iter_mut().find(|p| p.id == pid) {
                                                        for id in added {
                                                            if !pl.tracks.contains(&id) {
                                                                pl.tracks.push(id);
                                                            }
                                                        }
                                                    }
                                                }
                                            });
                                        }
                                    } else {
                                        let mut store = playlist_store.write();
                                        if let Some(playlist) = store.playlists.iter_mut().find(|p| p.id == playlist_id) {
                                            for path in tracks {
                                                if !playlist.tracks.contains(&path) {
                                                    playlist.tracks.push(path);
                                                }
                                            }
                                        }
                                    }
                                }
                                show_album_playlist_modal.set(false);
                            },
                            on_create_playlist: move |name: String| {
                                if let Some(aid) = pending_album_id_for_playlist.read().clone() {
                                    let tracks: Vec<_> = {
                                        let lib = library.read();
                                        if is_server {
                                            lib.jellyfin_tracks.iter()
                                                .filter(|t| t.album_id == aid)
                                                .map(|t| t.path.clone())
                                                .collect()
                                        } else {
                                            let album_title = lib.albums.iter()
                                                .find(|a| a.id == aid)
                                                .map(|a| a.title.clone());
                                            if let Some(title) = album_title {
                                                lib.tracks.iter()
                                                    .filter(|t| t.album == title)
                                                    .map(|t| t.path.clone())
                                                    .collect()
                                            } else {
                                                Vec::new()
                                            }
                                        }
                                    };
                                    if is_server {
                                        let playlist_name = name.clone();
                                        let paths = tracks.clone();
                                        let server_vals = {
                                            let conf = config.peek();
                                            conf.server.as_ref().and_then(|s| {
                                                if let (Some(tok), Some(uid)) = (&s.access_token, &s.user_id) {
                                                    Some((s.service, s.url.clone(), tok.clone(), uid.clone(), conf.device_id.clone()))
                                                } else { None }
                                            })
                                        };
                                        if let Some((service, url, token, user_id, device_id)) = server_vals {
                                            spawn(async move {
                                                let item_ids: Vec<String> = paths.iter()
                                                    .filter_map(|p| {
                                                        let parts: Vec<&str> = p.to_str()?.split(':').collect();
                                                        if parts.len() >= 2 { Some(parts[1].to_string()) } else { None }
                                                    })
                                                    .collect();
                                                let id_refs: Vec<&str> = item_ids.iter().map(|s| s.as_str()).collect();
                                                let result = match service {
                                                    MusicService::Jellyfin => {
                                                        let remote = JellyfinClient::new(&url, Some(&token), &device_id, Some(&user_id));
                                                        remote.create_playlist(&playlist_name, &id_refs).await
                                                    }
                                                    MusicService::Subsonic | MusicService::Custom => {
                                                        let remote = SubsonicClient::new(&url, &user_id, &token);
                                                        remote.create_playlist(&playlist_name, &id_refs).await
                                                    }
                                                };
                                                if let Ok(new_id) = result {
                                                    let mut store = playlist_store.write();
                                                    store.jellyfin_playlists.push(reader::models::JellyfinPlaylist {
                                                        id: new_id,
                                                        name: playlist_name,
                                                        tracks: item_ids,
                                                        image_tag: None,
                                                        cover_path: None,
                                                    });
                                                }
                                            });
                                        }
                                    } else {
                                        let mut store = playlist_store.write();
                                        store.playlists.push(reader::models::Playlist {
                                            id: uuid::Uuid::new_v4().to_string(),
                                            name,
                                            tracks,
                                            cover_path: None,
                                        });
                                    }
                                }
                                show_album_playlist_modal.set(false);
                            },
                        }
                    }
                }
            } else {
                if is_server {
                    ServerAlbumDetails {
                        album_jellyfin_id: album_id.read().clone(),
                        library,
                        config,
                        playlist_store,
                        queue,
                        on_close: move |_| album_id.set(String::new()),
                    }
                } else {
                    components::album_details::AlbumDetails {
                        album_id: album_id.read().clone(),
                        library,
                        playlist_store,
                        on_close: move |_| album_id.set(String::new()),
                    }
                }
            }
        }
    }
}
