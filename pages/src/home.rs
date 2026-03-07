use config::{AppConfig, MusicSource};
use dioxus::prelude::*;
use rand::seq::SliceRandom;
use rand::thread_rng;
use reader::{Library, PlaylistStore};
use server::jellyfin::JellyfinRemote;

#[component]
pub fn Home(
    library: Signal<Library>,
    playlist_store: Signal<PlaylistStore>,
    on_select_album: EventHandler<String>,
    on_play_album: EventHandler<String>,
    on_select_playlist: EventHandler<String>,
    on_search_artist: EventHandler<String>,
) -> Element {
    let config = use_context::<Signal<AppConfig>>();
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
                            if let Ok(genres) = remote.get_genres().await {
                                let mut lib_write = library.write();
                                lib_write.jellyfin_genres =
                                    genres.into_iter().map(|g| (g.name, g.id)).collect();
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
                && library.read().jellyfin_albums.is_empty()
            {
                fetch_jellyfin();
            } else {
                has_fetched_jellyfin.set(true);
            }
        }
    });

    let recent_albums = use_memo(move || {
        let lib = library.read();
        lib.albums
            .iter()
            .rev()
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
    });

    let recent_playlists = use_memo(move || {
        let store = playlist_store.read();
        let is_jelly = config.read().active_source == MusicSource::Jellyfin;
        if is_jelly {
            store
                .jellyfin_playlists
                .iter()
                .rev()
                .take(10)
                .cloned()
                .map(|p| {
                    (
                        p.id,
                        p.name,
                        p.tracks.len(),
                        p.tracks.first().cloned(),
                        true,
                    )
                })
                .collect::<Vec<_>>()
        } else {
            store
                .playlists
                .iter()
                .rev()
                .take(10)
                .cloned()
                .map(|p| {
                    (
                        p.id,
                        p.name,
                        p.tracks.len(),
                        p.tracks
                            .first()
                            .and_then(|p| p.to_str())
                            .map(|s| s.to_string()),
                        false,
                    )
                })
                .collect::<Vec<_>>()
        }
    });

    let artists = use_memo(move || {
        let lib = library.read();
        let mut unique_artists = std::collections::HashSet::new();
        let mut artist_list = Vec::new();

        for album in &lib.albums {
            if unique_artists.insert(album.artist.clone()) {
                let cover = album.cover_path.clone();
                artist_list.push((album.artist.clone(), cover));
            }
            if artist_list.len() >= 10 {
                break;
            }
        }
        artist_list
    });

    let jellyfin_albums_all = use_memo(move || {
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
                    cover_url,
                )
            })
            .collect::<Vec<_>>()
    });

    let jellyfin_shuffled = use_memo(move || {
        let albums = jellyfin_albums_all();
        if albums.is_empty() {
            return Vec::new();
        }

        let mut rng = thread_rng();
        let mut shuffled = albums.clone();
        shuffled.shuffle(&mut rng);
        shuffled
    });

    let local_shuffled = use_memo(move || {
        let lib = library.read();
        let mut albums = lib.albums.clone();
        if albums.is_empty() {
            return Vec::new();
        }

        let mut rng = thread_rng();
        albums.shuffle(&mut rng);
        albums
    });

    let jellyfin_albums_limited = use_memo(move || {
        jellyfin_albums_all()
            .into_iter()
            .take(20)
            .collect::<Vec<_>>()
    });

    let jellyfin_artists = use_memo(move || {
        let lib = library.read();
        let mut unique_artists = std::collections::HashSet::new();
        let mut artist_list = Vec::new();

        for track in &lib.jellyfin_tracks {
            if unique_artists.insert(track.artist.clone()) {
                let cover_url = if let Some(server) = &config.read().server {
                    let path_str = track.path.to_string_lossy();
                    let parts: Vec<&str> = path_str.split(':').collect();
                    if parts.len() >= 2 {
                        let id = parts[1];
                        let mut url = format!("{}/Items/{}/Images/Primary", server.url, id);
                        if let Some(token) = &server.access_token {
                            url.push_str(&format!("?api_key={}", token));
                        }
                        Some(url)
                    } else {
                        None
                    }
                } else {
                    None
                };
                artist_list.push((track.artist.clone(), cover_url));
            }
            if artist_list.len() >= 10 {
                break;
            }
        }
        artist_list
    });

    let scroll_container = move |id: &str, direction: i32| {
        let script = format!(
            "document.getElementById('{}').scrollBy({{ left: {}, behavior: 'smooth' }})",
            id,
            direction * 300
        );
        let _ = document::eval(&script);
    };

    rsx! {
        div {
            class: "p-8 space-y-12 pb-32 animate-fade-in w-full max-w-[1600px] mx-auto",

            div { class: "flex items-center justify-between mb-2",
                div {
                    h1 { class: "text-4xl font-black text-white tracking-tight", "Home" }
                }
            }

            section { class: "relative h-[350px] rounded-3xl overflow-hidden mb-12",
                if is_jellyfin {
                   {
                    let jelly_list = jellyfin_shuffled.read();
                    if let Some((album_id, title, artist, cover_url)) = jelly_list.first() {
                        rsx! {
                            div { class: "absolute inset-0",
                                if let Some(url) = cover_url {
                                    img { src: "{url}", class: "w-full h-full object-cover" }
                                }
                                div { class: "absolute inset-0 bg-gradient-to-r from-black/90 via-black/40 to-transparent" }
                            }
                            div { class: "relative h-full flex flex-col justify-center p-8 md:p-12",
                                span { class: "text-indigo-400 font-bold tracking-widest uppercase text-[10px] mb-3 flex items-center gap-2",
                                    i { class: "fa-solid fa-star text-[8px]" }
                                    "Featured Album"
                                }
                                h1 { class: "text-3xl md:text-5xl font-black text-white mb-4 leading-tight max-w-xl break-words", "{title}" }
                                p { class: "text-base md:text-lg text-white/60 mb-8 font-medium line-clamp-1 max-w-md", "By {artist}" }
                                div { class: "flex items-center gap-4",
                                    button {
                                        class: "flex items-center gap-3 bg-white text-black px-8 py-3 rounded-full font-bold hover:bg-white/90 hover:scale-105 active:scale-95 transition-all w-fit",
                                        onclick: {
                                            let id = album_id.clone();
                                            move |_| on_play_album.call(id.clone())
                                        },
                                        i { class: "fa-solid fa-play text-[10px]" }
                                        span { class: "text-sm", "Start Listening" }
                                    }
                                    button {
                                        class: "w-11 h-11 rounded-full bg-white/10 border border-white/20 flex items-center justify-center text-white hover:bg-white/20 transition-all",
                                        i { class: "fa-regular fa-heart" }
                                    }
                                }
                            }
                        }
                    } else { rsx! { div {} } }
                   }
                } else {
                   {
                    let local_list = local_shuffled.read();
                    if let Some(album) = local_list.first() {
                        rsx! {
                            div { class: "absolute inset-0",
                                if let Some(url) = utils::format_artwork_url(album.cover_path.as_ref()) {
                                    img { src: "{url}", class: "w-full h-full object-cover" }
                                }
                                div { class: "absolute inset-0 bg-gradient-to-r from-black/90 via-black/40 to-transparent" }
                            }
                            div { class: "relative h-full flex flex-col justify-center p-8 md:p-12",
                                span { class: "text-indigo-400 font-bold tracking-widest uppercase text-[10px] mb-3 flex items-center gap-2",
                                    i { class: "fa-solid fa-clock-rotate-left text-[8px]" }
                                    "Jump back in"
                                }
                                h1 { class: "text-3xl md:text-5xl font-black text-white mb-4 leading-tight max-w-xl break-words", "{album.title}" }
                                p { class: "text-base md:text-lg text-white/60 mb-8 font-medium line-clamp-1 max-w-md", "By {album.artist}" }
                                div { class: "flex items-center gap-4",
                                    button {
                                        class: "flex items-center gap-3 bg-white text-black px-8 py-3 rounded-full font-bold hover:bg-white/90 hover:scale-105 active:scale-95 transition-all w-fit",
                                        onclick: {
                                            let id = album.id.clone();
                                            move |_| on_play_album.call(id.clone())
                                        },
                                        i { class: "fa-solid fa-play text-[10px]" }
                                        span { class: "text-sm", "Start Listening" }
                                    }
                                    button {
                                        class: "w-11 h-11 rounded-full bg-white/10 border border-white/20 flex items-center justify-center text-white hover:bg-white/20 transition-all",
                                        i { class: "fa-regular fa-heart" }
                                    }
                                }
                            }
                        }
                    } else {
                        rsx! {
                            div { class: "absolute inset-0 bg-gradient-to-br from-indigo-900/40 to-purple-900/40 flex items-center justify-center",
                                div { class: "text-center",
                                    i { class: "fa-solid fa-music text-6xl text-white/10 mb-4" }
                                    h2 { class: "text-2xl font-bold text-white/40", "Add music to get started" }
                                }
                            }
                        }
                    }
                   }
                }
            }

            if is_jellyfin {
                {
                    let jelly_list = jellyfin_shuffled.read();
                    if !jelly_list.is_empty() {
                        rsx! {
                            section { class: "mb-12",
                                div { class: "flex items-end justify-between mb-6",
                                    div {
                                        h2 { class: "text-3xl font-extrabold text-white tracking-tight leading-none", "Listen Now" }
                                    }
                                }
                                div { class: "grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 gap-4",
                                    for (album_id, title, artist, cover_url) in jelly_list.iter().skip(1).take(8) {
                                        div {
                                            class: "flex items-center bg-white/5 hover:bg-white/10 border border-white/5 rounded-2xl cursor-pointer transition-all duration-300 group overflow-hidden pr-4",
                                            onclick: {
                                                let id = album_id.clone();
                                                move |_| on_select_album.call(id.clone())
                                            },
                                            div { class: "w-16 h-16 md:w-20 md:h-20 flex-shrink-0 bg-stone-800/50 relative overflow-hidden",
                                                if let Some(url) = cover_url {
                                                    img { src: "{url}", class: "w-full h-full object-cover group-hover:scale-105 transition-transform duration-500" }
                                                } else {
                                                     div { class: "w-full h-full flex items-center justify-center",
                                                        i { class: "fa-solid fa-compact-disc text-xl text-white/20" }
                                                     }
                                                }
                                                div { class: "absolute inset-0 bg-black/0 group-hover:bg-black/20 transition-colors duration-300" }
                                            }
                                            div { class: "p-4 flex-1 min-w-0 flex flex-col justify-center",
                                                h3 { class: "text-white font-bold truncate text-sm md:text-base", "{title}" }
                                                p { class: "text-xs text-white/50 truncate font-semibold mt-1", "{artist}" }
                                            }
                                            div { class: "opacity-0 group-hover:opacity-100 transition-all duration-300 translate-x-2 group-hover:translate-x-0",
                                                div { class: "w-8 h-8 rounded-full bg-white/10 flex items-center justify-center hover:bg-white/20 transition-colors",
                                                    onclick: {
                                                        let id = album_id.clone();
                                                        move |evt| {
                                                            evt.stop_propagation();
                                                            on_play_album.call(id.clone());
                                                        }
                                                    },
                                                    i { class: "fa-solid fa-play text-white/80 text-xs" }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    } else { rsx! { div {} } }
                }



                if !jellyfin_artists().is_empty() {
                    section { class: "mt-12",
                        div { class: "flex items-center justify-between mb-6",
                            h2 { class: "text-2xl font-bold text-white tracking-tight", "Top Artists" }
                            div { class: "flex gap-2",
                                button {
                                    class: "w-8 h-8 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-all hover:scale-105",
                                    onclick: move |_| scroll_container("jelly-artists-scroll", -1),
                                    i { class: "fa-solid fa-chevron-left text-sm" }
                                }
                                button {
                                    class: "w-8 h-8 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-all hover:scale-105",
                                    onclick: move |_| scroll_container("jelly-artists-scroll", 1),
                                    i { class: "fa-solid fa-chevron-right text-sm" }
                                }
                            }
                        }
                        div {
                            id: "jelly-artists-scroll",
                            class: "flex overflow-x-auto gap-6 pb-6 pt-2 overflow-y-visible scrollbar-hide scroll-smooth -mx-2 px-2",
                            for (artist, cover_url) in jellyfin_artists() {
                                div {
                                    class: "flex-none w-32 md:w-40 group cursor-pointer",
                                    onclick: {
                                        let artist = artist.clone();
                                        move |_| on_search_artist.call(artist.clone())
                                    },
                                    div { class: "w-32 h-32 md:w-40 md:h-40 rounded-full bg-stone-800/80 mb-4 overflow-hidden transition-all duration-500 relative mx-auto",
                                        if let Some(url) = cover_url {
                                            img { src: "{url}", class: "w-full h-full object-cover group-hover:scale-110 transition-transform duration-700" }
                                        } else {
                                             div { class: "w-full h-full flex items-center justify-center",
                                                i { class: "fa-solid fa-microphone text-4xl text-white/20" }
                                             }
                                        }
                                        div { class: "absolute inset-0 bg-black/0 group-hover:bg-black/20 transition-colors duration-300 rounded-full" }
                                    }
                                    h3 { class: "text-white font-bold truncate text-center px-2 text-sm md:text-base group-hover:text-indigo-400 transition-colors", "{artist}" }
                                }
                            }
                        }
                    }
                }

                if !jellyfin_albums_all().is_empty() {
                    section { class: "mt-12",
                        div { class: "flex items-center justify-between mb-6",
                             h2 { class: "text-2xl font-bold text-white tracking-tight", "New Releases" }
                             div { class: "flex gap-2",
                                button {
                                    class: "w-8 h-8 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-all hover:scale-105",
                                    onclick: move |_| scroll_container("jelly-albums-scroll", -1),
                                    i { class: "fa-solid fa-chevron-left text-sm" }
                                }
                                button {
                                    class: "w-8 h-8 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-all hover:scale-105",
                                    onclick: move |_| scroll_container("jelly-albums-scroll", 1),
                                    i { class: "fa-solid fa-chevron-right text-sm" }
                                }
                            }
                        }
                        div {
                            id: "jelly-albums-scroll",
                            class: "flex overflow-x-auto gap-5 pb-6 pt-2 overflow-y-visible scrollbar-hide scroll-smooth -mx-2 px-2",
                            for (album_id, title, artist, cover_url) in jellyfin_albums_limited() {
                                div {
                                   class: "flex-none w-36 md:w-48 group cursor-pointer",
                                   onclick: {
                                       let id = album_id.clone();
                                       move |_| on_select_album.call(id.clone())
                                   },
                                   div { class: "aspect-square rounded-2xl bg-stone-800/80 mb-4 overflow-hidden transition-all duration-300 relative",
                                        if let Some(url) = cover_url {
                                            img { src: "{url}", class: "w-full h-full object-cover group-hover:scale-105 transition-transform duration-500" }
                                        } else {
                                             div { class: "w-full h-full flex items-center justify-center border border-white/5 rounded-2xl",
                                                i { class: "fa-solid fa-compact-disc text-4xl text-white/20" }
                                             }
                                        }
                                        div { class: "absolute inset-0 bg-black/0 group-hover:bg-black/20 transition-colors duration-300" }

                                        div { class: "absolute right-3 bottom-3 w-10 h-10 bg-white text-black rounded-full flex items-center justify-center translate-y-4 opacity-0 group-hover:translate-y-0 group-hover:opacity-100 transition-all duration-300",
                                            onclick: {
                                                let id = album_id.clone();
                                                move |evt| {
                                                    evt.stop_propagation();
                                                    on_play_album.call(id.clone());
                                                }
                                            },
                                            i { class: "fa-solid fa-play ml-0.5 text-sm" }
                                        }
                                   }
                                   h3 { class: "text-white font-bold truncate text-sm md:text-base px-1", "{title}" }
                                   p { class: "text-xs md:text-sm text-white/50 truncate px-1 font-semibold mt-1", "{artist}" }
                                }
                            }
                        }
                    }
                }

            } else {
                {
                    let local_list = local_shuffled.read();
                    if !local_list.is_empty() {
                        rsx! {
                            section { class: "mb-12",
                                div { class: "flex items-end justify-between mb-6 text-white",
                                     div {
                                        h2 { class: "text-3xl font-extrabold tracking-tight", "Listen Now" }
                                    }
                                }
                                div { class: "grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 gap-4",
                                    for album in local_list.iter().skip(1).take(8) {
                                        div {
                                            class: "flex items-center bg-white/5 hover:bg-white/10 border border-white/5 rounded-2xl cursor-pointer transition-all duration-300 group overflow-hidden pr-4",
                                            onclick: {
                                                let id = album.id.clone();
                                                move |_| on_select_album.call(id.clone())
                                            },
                                            div { class: "w-16 h-16 md:w-20 md:h-20 flex-shrink-0 bg-stone-800/50 relative overflow-hidden",
                                                if let Some(url) = utils::format_artwork_url(album.cover_path.as_ref()) {
                                                    img { src: "{url}", class: "w-full h-full object-cover group-hover:scale-105 transition-transform duration-500" }
                                                } else {
                                                     div { class: "w-full h-full flex items-center justify-center",
                                                        i { class: "fa-solid fa-compact-disc text-xl text-white/20" }
                                                     }
                                                }
                                                div { class: "absolute inset-0 bg-black/0 group-hover:bg-black/20 transition-colors duration-300" }
                                            }
                                            div { class: "p-4 flex-1 min-w-0 flex flex-col justify-center",
                                                h3 { class: "text-white font-bold truncate text-sm md:text-base", "{album.title}" }
                                                p { class: "text-xs text-white/50 truncate font-semibold mt-1", "{album.artist}" }
                                            }
                                            div { class: "opacity-0 group-hover:opacity-100 transition-all duration-300",
                                                div { class: "w-8 h-8 rounded-full bg-white/10 flex items-center justify-center hover:bg-white/20 transition-colors",
                                                    onclick: {
                                                        let id = album.id.clone();
                                                        move |evt| {
                                                            evt.stop_propagation();
                                                            on_play_album.call(id.clone());
                                                        }
                                                    },
                                                    i { class: "fa-solid fa-play text-white/80 text-xs" }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    } else { rsx! { div {} } }
                }

                if !artists().is_empty() {
                    section { class: "mt-12",
                        div { class: "flex items-center justify-between mb-6",
                            h2 { class: "text-2xl font-bold text-white tracking-tight", "Top Artists" }
                            div { class: "flex gap-2",
                                button {
                                    class: "w-8 h-8 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-all",
                                    onclick: move |_| scroll_container("artists-scroll", -1),
                                    i { class: "fa-solid fa-chevron-left text-sm" }
                                }
                                button {
                                    class: "w-8 h-8 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-all",
                                    onclick: move |_| scroll_container("artists-scroll", 1),
                                    i { class: "fa-solid fa-chevron-right text-sm" }
                                }
                            }
                        }
                        div {
                            id: "artists-scroll",
                            class: "flex overflow-x-auto gap-6 pb-6 pt-2 overflow-y-visible scrollbar-hide scroll-smooth -mx-2 px-2",
                            for (artist, cover_path) in artists() {
                                div {
                                    class: "flex-none w-32 md:w-36 group cursor-pointer",
                                    onclick: {
                                        let artist = artist.clone();
                                        move |_| on_search_artist.call(artist.clone())
                                    },
                                    div { class: "aspect-square rounded-full bg-stone-800/80 mb-4 overflow-hidden transition-all duration-300 relative mx-auto",
                                        if let Some(path) = cover_path {
                                            if let Some(url) = utils::format_artwork_url(Some(&path)) {
                                                img { src: "{url}", class: "w-full h-full object-cover group-hover:scale-110 transition-transform duration-700" }
                                            }
                                        } else {
                                             div { class: "w-full h-full flex items-center justify-center border border-white/5 rounded-full",
                                                i { class: "fa-solid fa-microphone text-3xl text-white/20" }
                                             }
                                        }
                                        div { class: "absolute inset-0 bg-black/0 group-hover:bg-black/10 transition-colors duration-300 rounded-full" }
                                    }
                                    h3 { class: "text-white font-bold truncate text-center px-1 text-sm md:text-base", "{artist}" }
                                }
                            }
                        }
                    }
                }

                if !recent_albums().is_empty() {
                    section { class: "mt-12",
                        div { class: "flex items-center justify-between mb-6",
                             h2 { class: "text-2xl font-bold text-white tracking-tight", "New Releases" }
                             div { class: "flex gap-2",
                                button {
                                    class: "w-8 h-8 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-all",
                                    onclick: move |_| scroll_container("albums-scroll", -1),
                                    i { class: "fa-solid fa-chevron-left text-sm" }
                                }
                                button {
                                    class: "w-8 h-8 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-all",
                                    onclick: move |_| scroll_container("albums-scroll", 1),
                                    i { class: "fa-solid fa-chevron-right text-sm" }
                                }
                            }
                        }
                        div {
                            id: "albums-scroll",
                            class: "flex overflow-x-auto gap-5 pb-6 pt-2 scrollbar-hide scroll-smooth -mx-2 px-2",
                            for album in recent_albums() {
                                div {
                                   class: "flex-none w-36 md:w-44 group cursor-pointer",
                                   onclick: {
                                       let id = album.id.clone();
                                       move |_| on_select_album.call(id.clone())
                                   },
                                   div { class: "aspect-square rounded-2xl bg-stone-800/80 mb-4 overflow-hidden transition-all duration-300 relative",
                                        if let Some(url) = utils::format_artwork_url(album.cover_path.as_ref()) {
                                            img { src: "{url}", class: "w-full h-full object-cover group-hover:scale-105 transition-transform duration-500" }
                                        } else {
                                             div { class: "w-full h-full flex items-center justify-center border border-white/5 rounded-2xl",
                                                i { class: "fa-solid fa-compact-disc text-3xl text-white/20" }
                                             }
                                        }
                                        div { class: "absolute inset-0 bg-black/0 group-hover:bg-black/20 transition-colors duration-300" }

                                        div { class: "absolute right-3 bottom-3 w-10 h-10 bg-white text-black rounded-full flex items-center justify-center translate-y-4 opacity-0 group-hover:translate-y-0 group-hover:opacity-100 transition-all duration-300",
                                            onclick: {
                                                let id = album.id.clone();
                                                move |evt| {
                                                    evt.stop_propagation();
                                                    on_play_album.call(id.clone());
                                                }
                                            },
                                            i { class: "fa-solid fa-play text-xs ml-0.5" }
                                        }
                                   }
                                   h3 { class: "text-white font-bold truncate text-sm md:text-base px-1", "{album.title}" }
                                   p { class: "text-xs md:text-sm text-white/50 truncate px-1 font-semibold mt-1", "{album.artist}" }
                                }
                            }
                        }
                    }
                }
            }

            if !recent_playlists().is_empty() {
                section { class: "mt-16",
                    div { class: "flex items-center justify-between mb-6",
                         div {
                            h2 { class: "text-2xl font-bold text-white tracking-tight", "Playlists" }
                         }
                         div { class: "flex gap-2",
                            button {
                                class: "w-8 h-8 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-all",
                                onclick: move |_| scroll_container("playlists-scroll", -1),
                                i { class: "fa-solid fa-chevron-left text-sm" }
                            }
                            button {
                                class: "w-8 h-8 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-all",
                                onclick: move |_| scroll_container("playlists-scroll", 1),
                                i { class: "fa-solid fa-chevron-right text-sm" }
                            }
                        }
                    }
                    div {
                        id: "playlists-scroll",
                        class: "flex overflow-x-auto gap-6 pb-6 pt-2 scrollbar-hide scroll-smooth -mx-2 px-2",
                        for (id, name, track_count, first_track, is_jelly) in recent_playlists() {
                            {
                                let cover_url = if let Some(track_path_or_id) = first_track {
                                    let lib = library.peek();
                                    if is_jelly {
                                        lib.jellyfin_tracks.iter()
                                            .find(|t| t.path.to_string_lossy().contains(&track_path_or_id))
                                            .and_then(|t| {
                                                let conf = config.peek();
                                                if let Some(server) = &conf.server {
                                                    let path_str = t.path.to_string_lossy();
                                                    let parts: Vec<&str> = path_str.split(':').collect();
                                                    if parts.len() >= 2 {
                                                        let id = parts[1];
                                                        let mut url = format!("{}/Items/{}/Images/Primary", server.url, id);
                                                        if let Some(token) = &server.access_token {
                                                            url.push_str(&format!("?api_key={}", token));
                                                        }
                                                        Some(url)
                                                    } else {
                                                        None
                                                    }
                                                } else {
                                                    None
                                                }
                                            })
                                    } else {
                                        lib.tracks.iter()
                                            .find(|t| t.path.to_string_lossy() == track_path_or_id)
                                            .and_then(|t| {
                                                lib.albums.iter()
                                                    .find(|a| a.id == t.album_id)
                                                    .and_then(|a| a.cover_path.as_ref())
                                                    .and_then(|cp| utils::format_artwork_url(Some(cp)))
                                            })
                                    }
                                } else {
                                    None
                                };
                                 rsx! {
                                     div {
                                        class: "flex-none w-40 md:w-48 group cursor-pointer",
                                        onclick: {
                                            let id = id.clone();
                                            move |_| on_select_playlist.call(id.clone())
                                        },
                                        div { class: "aspect-square rounded-2xl bg-white/5 mb-4 overflow-hidden transition-all duration-500 relative",
                                             if let Some(url) = cover_url {
                                                 img { src: "{url}", class: "w-full h-full object-cover group-hover:scale-110 transition-transform duration-700" }
                                             } else {
                                                 div { class: "w-full h-full flex items-center justify-center bg-gradient-to-br from-indigo-600/20 to-purple-600/20 group-hover:scale-110 transition-transform duration-700",
                                                     i { class: "fa-solid fa-music text-5xl opacity-40 text-white" }
                                                 }
                                             }
                                             div { class: "absolute inset-0 bg-black/0 group-hover:bg-black/20 transition-colors duration-300" }
                                        }
                                        div {
                                            h3 { class: "text-white font-bold truncate text-sm md:text-base px-1 group-hover:text-indigo-400 transition-colors", "{name}" }
                                            p { class: "text-xs md:text-sm text-white/40 truncate px-1 font-semibold mt-1",
                                                 if is_jelly { "Jellyfin • " }
                                                 "{track_count} tracks"
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
}
