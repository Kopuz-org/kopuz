use ::server::jellyfin::JellyfinClient;
use ::server::subsonic::SubsonicClient;
use config::MusicService;
use config::AppConfig;
use dioxus::prelude::*;
use rand::seq::SliceRandom;
use rand::thread_rng;
use reader::{FavoritesStore, Library, PlaylistStore};

#[component]
pub fn JellyfinHome(
    library: Signal<Library>,
    playlist_store: Signal<PlaylistStore>,
    favorites_store: Signal<FavoritesStore>,
    on_select_album: EventHandler<String>,
    on_play_album: EventHandler<String>,
    on_select_playlist: EventHandler<String>,
    on_search_artist: EventHandler<String>,
) -> Element {
    let config = use_context::<Signal<AppConfig>>();
    let mut has_fetched = use_signal(|| false);

    let mut fetch_jellyfin = move || {
        has_fetched.set(true);
        spawn(async move {
            let _ = crate::server::subsonic_sync::sync_server_library(library, config, false).await;
        });
    };

    use_effect(move || {
        if !*has_fetched.read() {
            if library.read().jellyfin_tracks.is_empty()
                && library.read().jellyfin_albums.is_empty()
            {
                fetch_jellyfin();
            } else {
                has_fetched.set(true);
            }
        }
    });

    let jellyfin_albums_all = use_memo(move || {
        let lib = library.read();
        let conf = config.read();

        let mut albums = lib.jellyfin_albums.clone();
        albums.sort_by(|a, b| {
            a.title
                .trim()
                .to_lowercase()
                .cmp(&b.title.trim().to_lowercase())
        });

        let mut unique_albums = Vec::new();
        let mut seen_titles = std::collections::HashSet::new();

        for album in albums {
            if seen_titles.insert(album.title.trim().to_lowercase()) {
                unique_albums.push(album);
            }
        }

        unique_albums
            .into_iter()
            .map(|album| {
                let cover_url = if let Some(server) = &conf.server {
                    album.cover_path.as_ref().and_then(|cover_path| {
                        let path_str = cover_path.to_string_lossy();
                        utils::jellyfin_image::jellyfin_image_url_from_path(
                            &path_str,
                            &server.url,
                            server.access_token.as_deref(),
                            384,
                            80,
                        )
                    })
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

    let jellyfin_albums_limited = use_memo(move || {
        jellyfin_albums_all()
            .into_iter()
            .take(20)
            .collect::<Vec<_>>()
    });

    let jellyfin_artists = use_memo(move || {
        let lib = library.read();
        let conf = config.read();
        let mut unique_artists = std::collections::HashSet::new();
        let mut artist_list = Vec::new();
        for track in &lib.jellyfin_tracks {
            if unique_artists.insert(track.artist.clone()) {
                let cover_url = if let Some(server) = &conf.server {
                    let path_str = track.path.to_string_lossy();
                    utils::jellyfin_image::track_cover_url_with_album_fallback(
                        &path_str,
                        &track.album_id,
                        &server.url,
                        server.access_token.as_deref(),
                        320,
                        80,
                    )
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

    let recent_playlists = use_memo(move || {
        let store = playlist_store.read();
        store
            .jellyfin_playlists
            .iter()
            .rev()
            .take(10)
            .cloned()
            .map(|p| (p.id, p.name, p.tracks.len(), p.tracks.first().cloned()))
            .collect::<Vec<_>>()
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
            section { class: "relative h-[350px] rounded-3xl overflow-hidden mb-12",
                {
                    let jelly_list = jellyfin_shuffled.read();
                    if let Some((album_id, title, artist, cover_url)) = jelly_list.first() {
                        rsx! {
                            div { class: "absolute inset-0",
                                if let Some(url) = cover_url {
                                    img { src: "{url}", class: "w-full h-full object-cover", decoding: "async", loading: "lazy" }
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
                                    {
                                        let album_id_hero = album_id.clone();
                                        let jelly_hero_fav = {
                                            let lib = library.read();
                                            let store = favorites_store.read();
                                            let tracks: Vec<_> = lib.jellyfin_tracks.iter()
                                                .filter(|t| t.album_id == *album_id)
                                                .collect();
                                            !tracks.is_empty() && tracks.iter().all(|t| {
                                                let path_str = t.path.to_string_lossy();
                                                let parts: Vec<&str> = path_str.split(':').collect();
                                                parts.len() >= 2 && store.is_jellyfin_favorite(parts[1])
                                            })
                                        };
                                        let hero_heart_class = if jelly_hero_fav {
                                            "w-11 h-11 rounded-full bg-white/10 border border-white/20 flex items-center justify-center text-red-400 hover:bg-white/20 transition-all"
                                        } else {
                                            "w-11 h-11 rounded-full bg-white/10 border border-white/20 flex items-center justify-center text-white hover:bg-white/20 transition-all"
                                        };
                                        let hero_heart_icon = if jelly_hero_fav { "fa-solid fa-heart" } else { "fa-regular fa-heart" };
                                        rsx! {
                                            button {
                                                class: "{hero_heart_class}",
                                                onclick: move |_| {
                                                    let lib = library.read();
                                                    let tracks: Vec<_> = lib.jellyfin_tracks.iter()
                                                        .filter(|t| t.album_id == album_id_hero)
                                                        .cloned()
                                                        .collect();
                                                    drop(lib);
                                                    let new_fav = !jelly_hero_fav;
                                                    for track in &tracks {
                                                        let path_str = track.path.to_string_lossy().to_string();
                                                        let parts: Vec<&str> = path_str.split(':').collect();
                                                        if parts.len() >= 2 {
                                                            favorites_store.write().set_jellyfin(parts[1].to_string(), new_fav);
                                                        }
                                                    }
                                                    let track_ids: Vec<String> = tracks.iter().filter_map(|t| {
                                                        let path_str = t.path.to_string_lossy().to_string();
                                                        let parts: Vec<&str> = path_str.split(':').collect();
                                                        if parts.len() >= 2 { Some(parts[1].to_string()) } else { None }
                                                    }).collect();
                                                    spawn(async move {
                                                        let server_config = {
                                                            let conf = config.peek();
                                                            if let Some(server) = &conf.server {
                                                                if let (Some(token), Some(user_id)) = (&server.access_token, &server.user_id) {
                                                                    Some((
                                                                        server.service,
                                                                        server.url.clone(),
                                                                        token.clone(),
                                                                        user_id.clone(),
                                                                        conf.device_id.clone(),
                                                                    ))
                                                                } else { None }
                                                            } else { None }
                                                        };
                                                        if let Some((service, url, token, user_id, device_id)) = server_config {
                                                            for id in &track_ids {
                                                                let result = match service {
                                                                    MusicService::Jellyfin => {
                                                                        let remote = JellyfinClient::new(
                                                                            &url,
                                                                            Some(&token),
                                                                            &device_id,
                                                                            Some(&user_id),
                                                                        );
                                                                        if new_fav {
                                                                            remote.mark_favorite(id).await
                                                                        } else {
                                                                            remote.unmark_favorite(id).await
                                                                        }
                                                                    }
                                                                    MusicService::Subsonic | MusicService::Custom => {
                                                                        let remote = SubsonicClient::new(&url, &user_id, &token);
                                                                        if new_fav {
                                                                            remote.star(id).await
                                                                        } else {
                                                                            remote.unstar(id).await
                                                                        }
                                                                    }
                                                                };
                                                                if let Err(e) = result {
                                                                    eprintln!("Failed to sync favorite: {e}");
                                                                }
                                                            }
                                                        }
                                                    });
                                                },
                                                i { class: "{hero_heart_icon}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    } else {
                        rsx! { div {} }
                    }
                }
            }

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
                            div { class: "grid grid-cols-[repeat(auto-fill,minmax(350px,1fr))] gap-4",
                                for (album_id, title, artist, cover_url) in jelly_list.iter().skip(1).take(8) {
                                    div {
                                        class: "flex items-center bg-white/5 hover:bg-white/10 border border-white/5 rounded-2xl cursor-pointer transition-all duration-300 group overflow-hidden pr-4",
                                        onclick: {
                                            let id = album_id.clone();
                                            move |_| on_select_album.call(id.clone())
                                        },
                                        div { class: "w-16 h-16 md:w-20 md:h-20 flex-shrink-0 bg-stone-800/50 relative overflow-hidden",
                                            if let Some(url) = cover_url {
                                                img { src: "{url}", class: "w-full h-full object-cover", decoding: "async", loading: "lazy" }
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
                                            div {
                                                class: "w-8 h-8 rounded-full bg-white/10 flex items-center justify-center hover:bg-white/20 transition-colors",
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
                } else {
                    rsx! { div {} }
                }
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
                                        img { src: "{url}", class: "w-full h-full object-cover", decoding: "async", loading: "lazy" }
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
                                        img { src: "{url}", class: "w-full h-full object-cover", decoding: "async", loading: "lazy" }
                                    } else {
                                        div { class: "w-full h-full flex items-center justify-center border border-white/5 rounded-2xl",
                                            i { class: "fa-solid fa-compact-disc text-4xl text-white/20" }
                                        }
                                    }
                                    div { class: "absolute inset-0 bg-black/0 group-hover:bg-black/20 transition-colors duration-300" }
                                    div {
                                        class: "absolute right-3 bottom-3 w-10 h-10 bg-white text-black rounded-full flex items-center justify-center translate-y-4 opacity-0 group-hover:translate-y-0 group-hover:opacity-100 transition-all duration-300",
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

            if !recent_playlists().is_empty() {
                section { class: "mt-16",
                    div { class: "flex items-center justify-between mb-6",
                        div {
                            h2 { class: "text-2xl font-bold text-white tracking-tight", "Playlists" }
                        }
                        div { class: "flex gap-2",
                            button {
                                class: "w-8 h-8 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-all",
                                onclick: move |_| scroll_container("jelly-playlists-scroll", -1),
                                i { class: "fa-solid fa-chevron-left text-sm" }
                            }
                            button {
                                class: "w-8 h-8 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-all",
                                onclick: move |_| scroll_container("jelly-playlists-scroll", 1),
                                i { class: "fa-solid fa-chevron-right text-sm" }
                            }
                        }
                    }
                    div {
                        id: "jelly-playlists-scroll",
                        class: "flex overflow-x-auto gap-6 pb-6 pt-2 scrollbar-hide scroll-smooth -mx-2 px-2",
                        for (id, name, track_count, first_track_id) in recent_playlists() {
                            {
                                let cover_url = if let Some(tid) = first_track_id {
                                    let lib = library.peek();
                                    lib.jellyfin_tracks
                                        .iter()
                                        .find(|t| t.path.to_string_lossy().contains(&tid))
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
                                        class: "flex-none w-40 md:w-48 group cursor-pointer",
                                        onclick: {
                                            let id = id.clone();
                                            move |_| on_select_playlist.call(id.clone())
                                        },
                                        div { class: "aspect-square rounded-2xl bg-white/5 mb-4 overflow-hidden transition-all duration-500 relative",
                                            if let Some(url) = cover_url {
                                                img { src: "{url}", class: "w-full h-full object-cover", decoding: "async", loading: "lazy" }
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
                                                "Music • {track_count} tracks"
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

pub use JellyfinHome as ServerHome;
