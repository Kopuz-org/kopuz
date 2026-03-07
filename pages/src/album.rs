use config::{AppConfig, MusicSource};
use dioxus::prelude::*;
use player::player;
use reader::{Library, PlaylistStore};
use server::jellyfin::JellyfinRemote;

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
    let is_jellyfin = config.read().active_source == MusicSource::Jellyfin;

    let mut has_fetched_jellyfin = use_signal(|| false);

    let mut fetch_jellyfin = move || {
        has_fetched_jellyfin.set(true);
        spawn(async move {
            let conf = config.read();
            if let Some(server) = &conf.server {
                if let (Some(token), Some(user_id)) = (&server.access_token, &server.user_id) {
                    let remote = JellyfinRemote::new(
                        &server.url,
                        Some(token),
                        &conf.device_id,
                        Some(user_id),
                    );

                    if let Ok(libs) = remote.get_music_libraries().await {
                        for lib in libs {
                            let mut album_start_index = 0;
                            let album_limit = 100;
                            loop {
                                if let Ok((albums, _total)) = remote
                                    .get_albums_paginated(&lib.id, album_start_index, album_limit)
                                    .await
                                {
                                    if albums.is_empty() {
                                        break;
                                    }
                                    let count = albums.len();
                                    let mut new_albums = Vec::new();
                                    for album_item in albums {
                                        let image_tag = album_item
                                            .image_tags
                                            .as_ref()
                                            .and_then(|t| t.get("Primary").cloned());

                                        let cover_url = if image_tag.is_some() {
                                            Some(std::path::PathBuf::from(format!(
                                                "jellyfin:{}:{}",
                                                album_item.id,
                                                image_tag.as_ref().unwrap()
                                            )))
                                        } else {
                                            Some(std::path::PathBuf::from(format!(
                                                "jellyfin:{}",
                                                album_item.id
                                            )))
                                        };

                                        let album = reader::models::Album {
                                            id: format!("jellyfin:{}", album_item.id),
                                            title: album_item.name,
                                            artist: album_item
                                                .album_artist
                                                .or_else(|| {
                                                    album_item
                                                        .artists
                                                        .as_ref()
                                                        .map(|a| a.join(", "))
                                                })
                                                .unwrap_or_default(),
                                            genre: album_item
                                                .genres
                                                .as_ref()
                                                .map(|g| g.join(", "))
                                                .unwrap_or_default(),
                                            year: album_item.production_year.unwrap_or(0),
                                            cover_path: cover_url,
                                        };
                                        new_albums.push(album);
                                    }
                                    {
                                        let mut lib_write = library.write();
                                        for album in new_albums {
                                            if !lib_write
                                                .jellyfin_albums
                                                .iter()
                                                .any(|a| a.id == album.id)
                                            {
                                                lib_write.jellyfin_albums.push(album);
                                            }
                                        }
                                    }
                                    album_start_index += count;
                                    if count < album_limit {
                                        break;
                                    }
                                } else {
                                    break;
                                }
                            }

                            let mut start_index = 0;
                            let limit = 200;
                            loop {
                                if let Ok(items) = remote
                                    .get_music_library_items_paginated(&lib.id, start_index, limit)
                                    .await
                                {
                                    if items.is_empty() {
                                        break;
                                    }
                                    let count = items.len();
                                    let mut new_tracks = Vec::new();
                                    for item in items {
                                        let duration_secs =
                                            item.run_time_ticks.unwrap_or(0) / 10_000_000;
                                        let mut path_str = format!("jellyfin:{}", item.id);
                                        if let Some(tags) = &item.image_tags {
                                            if let Some(tag) = tags.get("Primary") {
                                                path_str.push_str(&format!(":{}", tag));
                                            }
                                        }

                                        let bitrate_kbps = item.bitrate.unwrap_or(0) / 1000;
                                        let bitrate_u8 = if bitrate_kbps > 255 {
                                            255
                                        } else {
                                            bitrate_kbps as u8
                                        };

                                        let sample_rate = item.sample_rate.unwrap_or(0);

                                        let track = reader::models::Track {
                                            path: std::path::PathBuf::from(path_str),
                                            album_id: item
                                                .album_id
                                                .map(|id| format!("jellyfin:{}", id))
                                                .unwrap_or_default(),
                                            title: item.name,
                                            artist: item
                                                .album_artist
                                                .or_else(|| item.artists.map(|a| a.join(", ")))
                                                .unwrap_or_default(),
                                            album: item.album.unwrap_or_default(),
                                            duration: duration_secs,
                                            khz: sample_rate,
                                            bitrate: bitrate_u8,
                                            track_number: item.index_number,
                                            disc_number: item.parent_index_number,
                                            musicbrainz_release_id: None,
                                        };
                                        new_tracks.push(track);
                                    }
                                    {
                                        let mut lib_write = library.write();
                                        for track in new_tracks {
                                            if !lib_write
                                                .jellyfin_tracks
                                                .iter()
                                                .any(|t| t.path == track.path)
                                            {
                                                lib_write.jellyfin_tracks.push(track);
                                            }
                                        }
                                    }
                                    start_index += count;
                                    if count < limit {
                                        break;
                                    }
                                } else {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        });
    };

    use_effect(move || {
        let is_jelly = config.read().active_source == MusicSource::Jellyfin;
        if is_jelly && !*has_fetched_jellyfin.read() {
            if library.read().jellyfin_tracks.is_empty()
                || library.read().jellyfin_albums.is_empty()
            {
                fetch_jellyfin();
            } else {
                has_fetched_jellyfin.set(true);
            }
        }
    });

    let local_albums = library.read().albums.clone();

    let jellyfin_albums = use_memo(move || {
        if !is_jellyfin {
            return Vec::new();
        }

        let lib = library.read();
        let conf = config.read();

        lib.jellyfin_albums
            .iter()
            .map(|album| {
                let cover_url = if let Some(server) = &conf.server {
                    if let Some(cover_path) = &album.cover_path {
                        let path_str = cover_path.to_string_lossy();
                        let parts: Vec<&str> = path_str.split(':').collect();
                        if parts.len() >= 2 {
                            let id = parts[1];
                            let mut url = format!("{}/Items/{}/Images/Primary", server.url, id);
                            let mut params = Vec::new();

                            if parts.len() >= 3 {
                                params.push(format!("tag={}", parts[2]));
                            }
                            if let Some(token) = &server.access_token {
                                params.push(format!("api_key={}", token));
                            }
                            if !params.is_empty() {
                                url.push('?');
                                url.push_str(&params.join("&"));
                            }
                            Some(url)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };

                (
                    album.id.clone(),
                    album.title.clone(),
                    album.artist.clone(),
                    album.genre.clone(),
                    album.year,
                    cover_url,
                )
            })
            .collect::<Vec<_>>()
    });

    rsx! {
        div {
            class: "p-8 pb-24",

            if album_id.read().is_empty() {
                div {
                    h1 { class: "text-3xl font-bold text-white mb-6", "All Albums" }

                    if is_jellyfin {
                        if jellyfin_albums().is_empty() {
                            p { class: "text-slate-500", "No albums found in Jellyfin library." }
                        } else {
                            div { class: "grid grid-cols-2 md:grid-cols-4 lg:grid-cols-5 gap-6",
                                for (album_id_val, album_title, artist, _genre, _year, cover_url) in jellyfin_albums() {
                                    div {
                                        key: "{album_id_val}",
                                        class: "group cursor-pointer p-4 bg-white/5 rounded-xl hover:bg-white/10 transition-colors",
                                        onclick: {
                                            let id = album_id_val.clone();
                                            move |_| album_id.set(id.clone())
                                        },
                                        div { class: "aspect-square rounded-lg bg-stone-800 mb-3 overflow-hidden relative",
                                            if let Some(url) = &cover_url {
                                                img { src: "{url}", class: "w-full h-full object-cover group-hover:scale-105 transition-transform duration-300" }
                                            } else {
                                                div { class: "w-full h-full flex items-center justify-center",
                                                    i { class: "fa-solid fa-compact-disc text-4xl text-white/20" }
                                                }
                                            }
                                        }
                                        h3 { class: "text-white font-medium truncate", "{album_title}" }
                                        p { class: "text-sm text-stone-400 truncate", "{artist}" }
                                    }
                                }
                            }
                        }
                    } else {
                        if local_albums.is_empty() {
                            p { class: "text-slate-500", "No albums found in library." }
                        } else {
                            div { class: "grid grid-cols-2 md:grid-cols-4 lg:grid-cols-5 gap-6",
                                for album in local_albums {
                                    div {
                                        key: "{album.id}",
                                        class: "group cursor-pointer p-4 bg-white/5 rounded-xl hover:bg-white/10 transition-colors",
                                        onclick: {
                                            let id = album.id.clone();
                                            move |_| album_id.set(id.clone())
                                        },
                                        div { class: "aspect-square rounded-lg bg-stone-800 mb-3 overflow-hidden relative",
                                            if let Some(url) = utils::format_artwork_url(album.cover_path.as_ref()) {
                                                img { src: "{url}", class: "w-full h-full object-cover group-hover:scale-105 transition-transform duration-300" }
                                            } else {
                                                div { class: "w-full h-full flex items-center justify-center",
                                                    i { class: "fa-solid fa-compact-disc text-4xl text-white/20" }
                                                }
                                            }
                                        }
                                        h3 { class: "text-white font-medium truncate", "{album.title}" }
                                        p { class: "text-sm text-stone-400 truncate", "{album.artist}" }
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                if is_jellyfin {
                    JellyfinAlbumDetails {
                        album_jellyfin_id: album_id.read().clone(),
                        library: library,
                        config: config,
                        playlist_store: playlist_store,
                        player: player,
                        is_playing: is_playing,
                        current_song_cover_url: current_song_cover_url,
                        current_song_title: current_song_title,
                        current_song_artist: current_song_artist,
                        current_song_duration: current_song_duration,
                        current_song_progress: current_song_progress,
                        queue: queue,
                        current_queue_index: current_queue_index,
                        on_close: move |_| album_id.set(String::new()),
                    }
                } else {
                    components::album_details::AlbumDetails {
                        album_id: album_id.read().clone(),
                        library: library,
                        playlist_store: playlist_store,
                        player: player,
                        is_playing: is_playing,
                        current_song_cover_url: current_song_cover_url,
                        current_song_title: current_song_title,
                        current_song_artist: current_song_artist,
                        current_song_duration: current_song_duration,
                        current_song_progress: current_song_progress,
                        queue: queue,
                        current_queue_index: current_queue_index,
                        on_close: move |_| album_id.set(String::new()),
                    }
                }
            }
        }
    }
}

#[component]
fn JellyfinAlbumDetails(
    album_jellyfin_id: String,
    library: Signal<Library>,
    config: Signal<AppConfig>,
    playlist_store: Signal<PlaylistStore>,
    player: Signal<player::Player>,
    mut is_playing: Signal<bool>,
    mut current_song_cover_url: Signal<String>,
    mut current_song_title: Signal<String>,
    mut current_song_artist: Signal<String>,
    mut current_song_duration: Signal<u64>,
    mut current_song_progress: Signal<u64>,
    mut queue: Signal<Vec<reader::models::Track>>,
    mut current_queue_index: Signal<usize>,
    on_close: EventHandler<()>,
) -> Element {
    let mut ctrl = use_context::<hooks::use_player_controller::PlayerController>();
    let mut active_menu_track = use_signal(|| None::<std::path::PathBuf>);
    let mut show_playlist_modal = use_signal(|| false);
    let mut selected_track_for_playlist = use_signal(|| None::<std::path::PathBuf>);

    let album_id_for_info = album_jellyfin_id.clone();
    let album_info = use_memo(move || {
        let lib = library.read();
        lib.jellyfin_albums
            .iter()
            .find(|a| a.id == album_id_for_info)
            .cloned()
    });

    let album_id_for_tracks = album_jellyfin_id.clone();
    let album_tracks = use_memo(move || {
        let lib = library.read();
        let conf = config.read();

        let mut tracks: Vec<_> = lib
            .jellyfin_tracks
            .iter()
            .filter(|t| t.album_id == album_id_for_tracks)
            .map(|t| {
                let cover_url = if let Some(server) = &conf.server {
                    let path_str = t.path.to_string_lossy();
                    let parts: Vec<&str> = path_str.split(':').collect();
                    if parts.len() >= 2 {
                        let id = parts[1];
                        let mut url = format!("{}/Items/{}/Images/Primary", server.url, id);
                        let mut params = Vec::new();

                        if parts.len() >= 3 {
                            params.push(format!("tag={}", parts[2]));
                        }
                        if let Some(token) = &server.access_token {
                            params.push(format!("api_key={}", token));
                        }
                        if !params.is_empty() {
                            url.push('?');
                            url.push_str(&params.join("&"));
                        }
                        Some(url)
                    } else {
                        None
                    }
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

    let cover_url = {
        let conf = config.read();
        if let Some(server) = &conf.server {
            album.as_ref().and_then(|a| {
                a.cover_path.as_ref().and_then(|cover_path| {
                    let path_str = cover_path.to_string_lossy();
                    let parts: Vec<&str> = path_str.split(':').collect();
                    if parts.len() >= 2 {
                        let id = parts[1];
                        let mut url = format!("{}/Items/{}/Images/Primary", server.url, id);
                        let mut params = Vec::new();

                        if parts.len() >= 3 {
                            params.push(format!("tag={}", parts[2]));
                        }
                        if let Some(token) = &server.access_token {
                            params.push(format!("api_key={}", token));
                        }
                        if !params.is_empty() {
                            url.push('?');
                            url.push_str(&params.join("&"));
                        }
                        Some(url)
                    } else {
                        None
                    }
                })
            })
        } else {
            None
        }
    };

    rsx! {
        div {
            class: "w-full max-w-[1600px] mx-auto",

            if *show_playlist_modal.read() {
                components::playlist_modal::PlaylistModal {
                    playlist_store: playlist_store,
                    is_jellyfin: true,
                    on_close: move |_| show_playlist_modal.set(false),
                    on_add_to_playlist: move |playlist_id: String| {
                        if let Some(path) = selected_track_for_playlist.read().clone() {
                            let path_clone = path.clone();
                            let pid = playlist_id.clone();
                            spawn(async move {
                                let conf = config.peek();
                                if let Some(server) = &conf.server {
                                    if let (Some(token), Some(user_id)) = (&server.access_token, &server.user_id) {
                                        let remote = JellyfinRemote::new(
                                            &server.url,
                                            Some(token),
                                            &conf.device_id,
                                            Some(user_id),
                                        );
                                        let parts: Vec<&str> = path_clone.to_str().unwrap_or_default().split(':').collect();
                                        if parts.len() >= 2 {
                                            let item_id = parts[1];
                                            let _ = remote.add_to_playlist(&pid, item_id).await;
                                        }
                                    }
                                }
                            });
                        }
                        show_playlist_modal.set(false);
                        active_menu_track.set(None);
                    },
                    on_create_playlist: move |name: String| {
                        if let Some(path) = selected_track_for_playlist.read().clone() {
                            let path_clone = path.clone();
                            let playlist_name = name.clone();
                            spawn(async move {
                                let conf = config.peek();
                                if let Some(server) = &conf.server {
                                    if let (Some(token), Some(user_id)) = (&server.access_token, &server.user_id) {
                                        let remote = JellyfinRemote::new(
                                            &server.url,
                                            Some(token),
                                            &conf.device_id,
                                            Some(user_id),
                                        );
                                        let parts: Vec<&str> = path_clone.to_str().unwrap_or_default().split(':').collect();
                                        if parts.len() >= 2 {
                                            let item_id = parts[1];
                                            let _ = remote.create_playlist(&playlist_name, &[item_id]).await;
                                        }
                                    }
                                }
                            });
                        }
                        show_playlist_modal.set(false);
                        active_menu_track.set(None);
                    }
                }
            }

            div { class: "flex items-center justify-between mb-8",
                button {
                    class: "flex items-center gap-2 text-slate-400 hover:text-white transition-colors",
                    onclick: move |_| on_close.call(()),
                    i { class: "fa-solid fa-arrow-left" }
                    "Back to Albums"
                }
            }

            div {
                class: "flex flex-col md:flex-row items-end gap-8 mb-12",
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
                        h5 { class: "text-sm font-bold tracking-widest text-white/60 uppercase mb-2", "{artist}" }
                    }
                    h1 { class: "text-5xl md:text-7xl font-bold text-white mb-6", "{album_title}" }
                    div { class: "flex items-center gap-6 text-slate-400",
                        p { "{album_tracks().len()} songs" }
                        span { "•" }
                        p { "{duration_min} min" }
                    }
                }

                div { class: "flex items-center gap-4",
                    if !album_tracks().is_empty() {
                        button {
                            class: "w-14 h-14 rounded-full bg-indigo-500 hover:bg-indigo-400 text-black flex items-center justify-center transition-transform hover:scale-105",
                            onclick: {
                                let tracks_for_play: Vec<reader::models::Track> = album_tracks().iter().map(|(t, _)| t.clone()).collect();
                                move |_| {
                                    queue.set(tracks_for_play.clone());
                                    ctrl.play_track(0);
                                }
                            },
                            i { class: "fa-solid fa-play text-xl ml-1" }
                        }
                    }
                }
            }

            div { class: "space-y-1",
                if album_tracks().is_empty() {
                    div { class: "py-12 flex flex-col items-center justify-center text-slate-600",
                        i { class: "fa-regular fa-folder-open text-4xl mb-4" }
                        p { class: "text-lg", "No songs here." }
                    }
                } else {
                    div { class: "grid grid-cols-[auto_1fr_1fr_auto_auto] gap-4 px-4 py-2 border-b border-white/5 text-sm font-medium text-slate-500 mb-2 uppercase tracking-wider",
                        div { class: "w-8 text-center", "#" }
                        div { "Title" }
                        div { "Album" }
                    }

                    for (idx, (track, track_cover_url)) in album_tracks().into_iter().enumerate() {
                        {
                            let track_key = track.path.display().to_string();
                            let track_menu = track.clone();
                            let track_add = track.clone();
                            let is_menu_open = active_menu_track.read().as_ref() == Some(&track.path);
                            let album_queue: Vec<reader::models::Track> = album_tracks().iter().map(|(t, _)| t.clone()).collect();

                            rsx! {
                                components::track_row::TrackRow {
                                    key: "{track_key}",
                                    track: track.clone(),
                                    cover_url: track_cover_url,
                                    is_menu_open: is_menu_open,
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
                                    on_delete: move |_| {
                                        active_menu_track.set(None);
                                    },
                                    on_play: move |_| {
                                        queue.set(album_queue.clone());
                                        ctrl.play_track(idx);
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
