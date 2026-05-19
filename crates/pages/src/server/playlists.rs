use ::server::jellyfin::JellyfinClient;
use ::server::subsonic::SubsonicClient;
use crate::server::download_manager::{DownloadQueue, DownloadStatus, queue_downloads};
use config::{AppConfig, MusicService};
use dioxus::prelude::*;
use reader::{Library, PlaylistStore};

#[component]
pub fn JellyfinPlaylists(
    playlist_store: Signal<PlaylistStore>,
    library: Signal<Library>,
    config: Signal<AppConfig>,
    mut selected_playlist_id: Signal<Option<String>>,
    #[props(default)] refresh_trigger: Signal<u64>,
) -> Element {
    let is_offline = use_context::<Signal<bool>>();
    let mut last_fetch_key = use_signal(|| None::<String>);
    let mut fetch_request_id = use_signal(|| 0u64);
    let download_queue = use_context::<Signal<DownloadQueue>>();

    use_effect(move || {
        let fetch_context = {
            let conf = config.read();
            conf.server.as_ref().and_then(|server| {
                if let (Some(token), Some(user_id)) = (&server.access_token, &server.user_id) {
                    Some((
                        server.service,
                        server.url.clone(),
                        token.clone(),
                        user_id.clone(),
                        conf.device_id.clone(),
                    ))
                } else {
                    None
                }
            })
        };

        let trigger = *refresh_trigger.read();

        // Build a "server identity" key (without trigger) to detect server changes
        let server_key = fetch_context
            .as_ref()
            .map(|(service, url, _, user_id, _)| format!("{service:?}|{url}|{user_id}"));

        // Build the full fetch key that also includes the trigger
        let fetch_key = fetch_context
            .as_ref()
            .map(|(service, url, token, user_id, _)| {
                format!("{service:?}|{url}|{user_id}|{token}|{trigger}")
            });

        // If we already have cached playlists for this server AND the trigger hasn't changed,
        // show the cached data without re-fetching.
        let has_cached = {
            let store = playlist_store.read();
            !store.jellyfin_playlists.is_empty()
        };
        let last_key = last_fetch_key.read().clone();

        // Extract the server-identity part of the last fetch key (everything before the last |)
        let last_server_key = last_key.as_ref().and_then(|k| {
            let parts: Vec<&str> = k.splitn(5, '|').collect();
            if parts.len() >= 3 {
                Some(format!("{}", &parts[..3].join("|")))
            } else { None }
        });

        // Skip if same key (already fetched this exact state)
        if last_key.as_ref() == fetch_key.as_ref() {
            return;
        }

        // If server identity is the same and we have cached data, only re-fetch on explicit trigger
        if server_key == last_server_key && has_cached && trigger == 0 {
            // Update the key so we don't keep hitting this branch, but don't fetch
            last_fetch_key.set(fetch_key.clone());
            return;
        }

        last_fetch_key.set(fetch_key.clone());

        let request_id = *fetch_request_id.read() + 1;
        fetch_request_id.set(request_id);

        let Some((service, url, token, user_id, device_id)) = fetch_context else {
            return;
        };

        spawn(async move {
            let mut server_playlists = Vec::new();

            match service {
                MusicService::Jellyfin => {
                    let remote =
                        JellyfinClient::new(&url, Some(&token), &device_id, Some(&user_id));
                    if let Ok(playlists) = remote.get_playlists().await {
                        for p in playlists {
                            let image_tag = p
                                .image_tags
                                .as_ref()
                                .and_then(|tags| tags.get("Primary"))
                                .cloned();
                            if let Ok(items) = remote.get_playlist_items(&p.id).await {
                                let tracks: Vec<String> =
                                    items.into_iter().map(|item| item.id).collect();
                                server_playlists.push(reader::models::JellyfinPlaylist {
                                    id: p.id.clone(),
                                    name: p.name.clone(),
                                    tracks,
                                    image_tag,
                                    cover_path: None,
                                });
                            } else {
                                server_playlists.push(reader::models::JellyfinPlaylist {
                                    id: p.id.clone(),
                                    name: p.name.clone(),
                                    tracks: vec![],
                                    image_tag,
                                    cover_path: None,
                                });
                            }
                        }
                    }
                }
                MusicService::Subsonic | MusicService::Custom => {
                    let remote = SubsonicClient::new(&url, &user_id, &token);
                    if let Ok(playlists) = remote.get_playlists().await {
                        for p in playlists {
                            let tracks = remote
                                .get_playlist_entries(&p.id)
                                .await
                                .unwrap_or_default()
                                .into_iter()
                                .map(|song| song.id)
                                .collect();
                            server_playlists.push(reader::models::JellyfinPlaylist {
                                id: p.id,
                                name: p.name,
                                tracks,
                                image_tag: None,
                                cover_path: None,
                            });
                        }
                    }
                }
            }

            if *fetch_request_id.read() != request_id {
                return;
            }

            let mut store_write = playlist_store.write();
            // Preserve any locally-set cover_path when replacing server data
            for p in &mut server_playlists {
                if let Some(existing) = store_write.jellyfin_playlists.iter().find(|e| e.id == p.id) {
                    p.cover_path = existing.cover_path.clone();
                }
            }
            store_write.jellyfin_playlists = server_playlists;
        });
    });

    let jellyfin_playlists = use_memo(move || {
        let store_ref = playlist_store.read();
        let offline = *is_offline.read();
        let conf = config.read();
        if offline {
            store_ref.jellyfin_playlists.iter().filter(|p| {
                !p.tracks.is_empty() && p.tracks.iter().all(|tid| {
                    if let Some(path_str) = conf.offline_tracks.get(tid) {
                        std::path::Path::new(path_str).exists()
                    } else {
                        false
                    }
                })
            }).cloned().collect()
        } else {
            store_ref.jellyfin_playlists.clone()
        }
    });

    let playlists = jellyfin_playlists.read().clone();

    rsx! {
        div {
            if playlists.is_empty() {
                div { class: "flex flex-col items-center justify-center h-64 text-slate-500",
                    i { class: "fa-regular fa-folder-open text-4xl mb-4 opacity-50" }
                    p { "{i18n::t(\"no_playlists_found\")}" }
                }
            } else {
                div { class: "grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 gap-6",
                    {playlists.into_iter().map(|playlist| {
                        let cover_url = {
                            let conf = config.peek();
                            if let Some(server) = &conf.server {
                                if let Some(path) = &playlist.cover_path {
                                    utils::format_artwork_url(Some(path))
                                } else if let Some(tag) = &playlist.image_tag {
                                    utils::map_cover_url(Some(utils::jellyfin_image::jellyfin_image_url(
                                        &server.url,
                                        &playlist.id,
                                        Some(tag.as_str()),
                                        server.access_token.as_deref(),
                                        384,
                                        80,
                                    )))
                                } else if let Some(first_track_id) = playlist.tracks.first() {
                                    let lib = library.peek();
                                    lib.jellyfin_tracks
                                        .iter()
                                        .find(|t| t.path.to_string_lossy().contains(first_track_id.as_str()))
                                        .and_then(|t| {
                                            let path_str = t.path.to_string_lossy();
                                            utils::map_cover_url(utils::jellyfin_image::track_cover_url_with_album_fallback(
                                                &path_str,
                                                &t.album_id,
                                                &server.url,
                                                server.access_token.as_deref(),
                                                384,
                                                80,
                                            ))
                                        })
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        };

                        let playlist_id_nav = playlist.id.clone();
                        let track_requests_dl: Vec<(String, String, String)> = {
                            let lib = library.peek();
                            playlist.tracks.iter().map(|tid| {
                                let meta = lib.jellyfin_tracks.iter()
                                    .find(|t| t.path.to_string_lossy().contains(tid.as_str()));
                                (
                                    tid.clone(),
                                    meta.map(|t| t.title.clone()).unwrap_or_default(),
                                    meta.map(|t| t.artist.clone()).unwrap_or_default(),
                                )
                            }).collect()
                        };
                        let is_dl = {
                            let q = download_queue.read();
                            playlist.tracks.iter().any(|tid| q.items.iter().any(|i| &i.id == tid && matches!(i.status, DownloadStatus::Queued | DownloadStatus::Downloading)))
                        };

                        let all_downloaded = !playlist.tracks.is_empty() && playlist.tracks.iter().all(|tid| {
                            if let Some(path_str) = config.read().offline_tracks.get(tid) {
                                std::path::Path::new(path_str).exists()
                            } else {
                                false
                            }
                        });

                        rsx! {
                            div {
                                key: "{playlist.id}",
                                class: "bg-white/5 border border-white/5 rounded-2xl p-6 hover:bg-white/10 transition-all cursor-pointer group relative",
                                onclick: move |_| selected_playlist_id.set(Some(playlist_id_nav.clone())),
                                div {
                                    class: "mb-4 w-full aspect-square rounded-xl flex items-center justify-center overflow-hidden transition-all bg-white/5",
                                    if let Some(url) = cover_url {
                                        img {
                                            src: "{url}",
                                            class: "w-full h-full object-cover",
                                            decoding: "async", loading: "lazy"
                                        }
                                    } else {
                                        div {
                                            class: "w-full h-full flex items-center justify-center",
                                            style: "background: color-mix(in srgb, var(--color-indigo-500), transparent 80%); color: var(--color-indigo-400)",
                                            i { class: "fa-solid fa-server text-2xl" }
                                        }
                                    }
                                }
                                h3 { class: "text-xl font-bold text-white mb-1 truncate", "{playlist.name}" }
                                p { class: "text-sm text-slate-400", "Server • {playlist.tracks.len()} tracks" }

                                button {
                                    class: "absolute top-4 right-4 w-8 h-8 rounded-full bg-black/40 border border-white/10 flex items-center justify-center text-white/60 hover:text-white hover:border-white/30 transition-colors opacity-0 group-hover:opacity-100",
                                    title: if all_downloaded { "Remove downloads" } else { "Download playlist for offline playback" },
                                    disabled: is_dl,
                                    onclick: move |evt| {
                                        evt.stop_propagation();
                                        if all_downloaded {
                                            let ids_only = playlist.tracks.clone();
                                            crate::server::download_manager::delete_downloads(ids_only, config, download_queue);
                                        } else {
                                            queue_downloads(track_requests_dl.clone(), config, download_queue);
                                        }
                                    },
                                    if is_dl {
                                        i { class: "fa-solid fa-spinner fa-spin text-xs" }
                                    } else if all_downloaded {
                                        i { class: "fa-solid fa-trash text-xs" }
                                    } else {
                                        i { class: "fa-solid fa-download text-xs" }
                                    }
                                }
                            }
                        }
                    })}
                }
            }
        }
    }
}

pub use JellyfinPlaylists as ServerPlaylists;
