use config::{AppConfig, MusicSource};
use dioxus::prelude::*;
use player::player;
use reader::{Library, PlaylistStore};

use crate::local::album::LocalAlbum;
use crate::server::album::{ServerAlbum, ServerAlbumDetails};

#[component]
pub fn Album(
    library: Signal<Library>,
    config: Signal<AppConfig>,
    album_id: Signal<String>,
    playlist_store: Signal<PlaylistStore>,
    player: Signal<player::Player>,
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
                    h1 { class: "text-3xl font-bold text-white mb-6", "All Albums" }

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
                                    let lib = library.read();
                                    let tracks: Vec<_> = if is_server {
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
                                    };
                                    let mut store = playlist_store.write();
                                    if let Some(playlist) = store.playlists.iter_mut().find(|p| p.id == playlist_id) {
                                        for path in tracks {
                                            if !playlist.tracks.contains(&path) {
                                                playlist.tracks.push(path);
                                            }
                                        }
                                    }
                                }
                                show_album_playlist_modal.set(false);
                            },
                            on_create_playlist: move |name: String| {
                                if let Some(aid) = pending_album_id_for_playlist.read().clone() {
                                    let lib = library.read();
                                    let tracks: Vec<_> = if is_server {
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
                                    };
                                    let mut store = playlist_store.write();
                                    store.playlists.push(reader::models::Playlist {
                                        id: uuid::Uuid::new_v4().to_string(),
                                        name,
                                        tracks,
                                    });
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
                        player,
                        is_playing,
                        current_song_cover_url,
                        current_song_title,
                        current_song_artist,
                        current_song_duration,
                        current_song_progress,
                        queue,
                        current_queue_index,
                        on_close: move |_| album_id.set(String::new()),
                    }
                }
            }
        }
    }
}
