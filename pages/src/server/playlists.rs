use ::server::jellyfin::JellyfinClient;
use ::server::subsonic::SubsonicClient;
use config::{AppConfig, MusicService};
use dioxus::prelude::*;
use reader::{Library, PlaylistStore};

#[component]
pub fn JellyfinPlaylists(
    playlist_store: Signal<PlaylistStore>,
    library: Signal<Library>,
    config: Signal<AppConfig>,
    mut selected_playlist_id: Signal<Option<String>>,
) -> Element {
    let mut last_fetch_key = use_signal(|| None::<String>);
    let mut fetch_request_id = use_signal(|| 0u64);

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

        let fetch_key = fetch_context
            .as_ref()
            .map(|(service, url, token, user_id, _)| {
                format!("{service:?}|{url}|{user_id}|{token}")
            });

        if *last_fetch_key.read() == fetch_key {
            return;
        }

        last_fetch_key.set(fetch_key.clone());

        {
            let mut store_write = playlist_store.write();
            store_write.jellyfin_playlists.clear();
        }

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
                            if let Ok(items) = remote.get_playlist_items(&p.id).await {
                                let tracks: Vec<String> =
                                    items.into_iter().map(|item| item.id).collect();
                                server_playlists.push(reader::models::JellyfinPlaylist {
                                    id: p.id.clone(),
                                    name: p.name.clone(),
                                    tracks,
                                });
                            } else {
                                server_playlists.push(reader::models::JellyfinPlaylist {
                                    id: p.id.clone(),
                                    name: p.name.clone(),
                                    tracks: vec![],
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
                            });
                        }
                    }
                }
            }

            if *fetch_request_id.read() != request_id {
                return;
            }

            let mut store_write = playlist_store.write();
            store_write.jellyfin_playlists = server_playlists;
        });
    });

    let store = playlist_store.read();

    rsx! {
        div {
            if store.jellyfin_playlists.is_empty() {
                div { class: "flex flex-col items-center justify-center h-64 text-slate-500",
                    i { class: "fa-regular fa-folder-open text-4xl mb-4 opacity-50" }
                    p { "No playlists found." }
                }
            } else {
                div { class: "grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 gap-6",
                    {store.jellyfin_playlists.iter().map(|playlist| {
                        let cover_url = if let Some(first_track_id) = playlist.tracks.first() {
                            let lib = library.peek();
                            lib.jellyfin_tracks
                                .iter()
                                .find(|t| t.path.to_string_lossy().contains(first_track_id.as_str()))
                                .and_then(|t| {
                                    let conf = config.peek();
                                    if let Some(server) = &conf.server {
                                        let path_str = t.path.to_string_lossy();
                                        utils::jellyfin_image::track_cover_url_with_album_fallback(
                                            &path_str,
                                            &t.album_id,
                                            &server.url,
                                            server.access_token.as_deref(),
                                            384,
                                            80,
                                        )
                                    } else {
                                        None
                                    }
                                })
                        } else {
                            None
                        };

                        rsx! {
                            div {
                                key: "{playlist.id}",
                                class: "bg-white/5 border border-white/5 rounded-2xl p-6 hover:bg-white/10 transition-all cursor-pointer group",
                                onclick: {
                                    let id = playlist.id.clone();
                                    move |_| selected_playlist_id.set(Some(id.clone()))
                                },
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
                            }
                        }
                    })}
                }
            }
        }
    }
}

pub use JellyfinPlaylists as ServerPlaylists;
