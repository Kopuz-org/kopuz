use ::server::jellyfin::JellyfinClient;
use ::server::subsonic::SubsonicClient;
use config::{AppConfig, ListenNowStyle, MusicService, UiStyle};
use dioxus::prelude::*;
use rand::seq::SliceRandom;
use rand::thread_rng;
use reader::{Album, FavoritesStore, Library, PlaylistStore, Track};
use std::collections::HashMap;

type AlbumCard = (String, String, String, Option<String>);

fn is_unknown_artist(value: &str) -> bool {
    let normalized = value.trim().to_lowercase();
    normalized.is_empty() || normalized == "unknown artist"
}

fn is_unknown_album(value: &str) -> bool {
    let normalized = value.trim().to_lowercase();
    normalized.is_empty() || normalized == "unknown album"
}

fn section_label(key: &str) -> String {
    let i18n_key = match key {
        "hero" => "home_section_hero",
        "continue_listening" => "home_section_continue_listening",
        "listen_now" => "home_section_listen_now",
        "top_artists" => "home_section_top_artists",
        "new_releases" => "home_section_new_releases",
        "made_for_you" => "home_section_made_for_you",
        "recently_added" => "home_section_recently_added",
        "playlists" => "home_section_playlists",
        _ => return key.to_string(),
    };
    i18n::t(i18n_key).to_string()
}

fn server_track_id(path: &str) -> Option<String> {
    let mut parts = path.split(':');
    let prefix = parts.next()?;
    let id = parts.next()?;
    if prefix == "jellyfin" || prefix == "subsonic" {
        Some(id.to_string())
    } else {
        None
    }
}

fn album_cover_url(conf: &AppConfig, album: &Album) -> Option<String> {
    let server = conf.server.as_ref()?;
    let cover_path = album.cover_path.as_ref()?;
    utils::jellyfin_image::jellyfin_image_url_from_path(
        &cover_path.to_string_lossy(),
        &server.url,
        server.access_token.as_deref(),
        384,
        80,
    )
}

fn track_cover_url(conf: &AppConfig, track: &Track) -> Option<String> {
    let server = conf.server.as_ref()?;
    let path_str = track.path.to_string_lossy();
    utils::jellyfin_image::track_cover_url_with_album_fallback(
        &path_str,
        &track.album_id,
        &server.url,
        server.access_token.as_deref(),
        384,
        80,
    )
}

#[component]
pub fn JellyfinHome(
    library: Signal<Library>,
    playlist_store: Signal<PlaylistStore>,
    favorites_store: Signal<FavoritesStore>,
    edit_mode: Signal<bool>,
    on_select_album: EventHandler<String>,
    on_play_album: EventHandler<String>,
    on_select_playlist: EventHandler<String>,
    on_search_artist: EventHandler<String>,
) -> Element {
    let is_offline = use_context::<Signal<bool>>();
    let mut config = use_context::<Signal<AppConfig>>();
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

    let jellyfin_albums_all = use_memo(move || -> Vec<AlbumCard> {
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

        let offline = *is_offline.read();
        let downloaded_album_ids: std::collections::HashSet<String> = if offline {
            lib.jellyfin_tracks
                .iter()
                .filter(|t| {
                    let id = t.path.to_string_lossy();
                    let id_str = id.split(':').nth(1).unwrap_or(&id);
                    if let Some(path_str) = conf.offline_tracks.get(id_str) {
                        std::path::Path::new(path_str).exists()
                    } else {
                        false
                    }
                })
                .map(|t| t.album_id.clone())
                .collect()
        } else {
            std::collections::HashSet::new()
        };

        for album in albums {
            if is_unknown_album(&album.title) || is_unknown_artist(&album.artist) {
                continue;
            }
            if offline && !downloaded_album_ids.contains(&album.id) {
                continue;
            }
            if seen_titles.insert(album.title.trim().to_lowercase()) {
                unique_albums.push(album);
            }
        }

        unique_albums
            .into_iter()
            .map(|album| {
                let cover = album_cover_url(&conf, &album);
                (
                    album.id.clone(),
                    album.title.clone(),
                    album.artist.clone(),
                    cover,
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

    let new_releases = use_memo(move || -> Vec<AlbumCard> {
        let lib = library.read();
        let conf = config.read();
        let mut albums = lib.jellyfin_albums.clone();
        albums.sort_by(|a, b| b.year.cmp(&a.year));
        let mut unique = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for album in albums {
            if is_unknown_album(&album.title) || is_unknown_artist(&album.artist) {
                continue;
            }
            if seen.insert(album.title.trim().to_lowercase()) {
                unique.push(album);
            }
            if unique.len() >= 12 {
                break;
            }
        }
        unique
            .into_iter()
            .map(|album| {
                let cover = album_cover_url(&conf, &album);
                (
                    album.id.clone(),
                    album.title.clone(),
                    album.artist.clone(),
                    cover,
                )
            })
            .collect()
    });

    let recently_added = use_memo(move || -> Vec<AlbumCard> {
        let lib = library.read();
        let conf = config.read();
        let mut unique = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for album in lib.jellyfin_albums.iter().rev() {
            if is_unknown_album(&album.title) || is_unknown_artist(&album.artist) {
                continue;
            }
            if seen.insert(album.title.trim().to_lowercase()) {
                unique.push(album.clone());
            }
            if unique.len() >= 12 {
                break;
            }
        }
        unique
            .into_iter()
            .map(|album| {
                let cover = album_cover_url(&conf, &album);
                (
                    album.id.clone(),
                    album.title.clone(),
                    album.artist.clone(),
                    cover,
                )
            })
            .collect()
    });

    let continue_listening = use_memo(move || {
        let lib = library.read();
        let conf = config.read();
        let track_by_id: HashMap<String, &Track> = lib
            .jellyfin_tracks
            .iter()
            .filter_map(|t| server_track_id(&t.path.to_string_lossy()).map(|id| (id, t)))
            .collect();
        let album_by_id: HashMap<&str, &Album> = lib
            .jellyfin_albums
            .iter()
            .map(|a| (a.id.as_str(), a))
            .collect();
        let mut out: Vec<(Track, Option<Album>, Option<String>)> = Vec::new();
        let mut seen_albums = std::collections::HashSet::new();
        for id in conf.recently_played_server.iter() {
            if let Some(track) = track_by_id.get(id) {
                let album = album_by_id.get(track.album_id.as_str()).copied().cloned();
                if let Some(ref album_ref) = album {
                    if is_unknown_album(&album_ref.title) || is_unknown_artist(&album_ref.artist) {
                        continue;
                    }
                } else if is_unknown_artist(&track.artist) && track.title.trim().is_empty() {
                    continue;
                }
                if let Some(ref a) = album {
                    if !seen_albums.insert(a.id.clone()) {
                        continue;
                    }
                }
                let cover = track_cover_url(&conf, track);
                out.push(((*track).clone(), album, cover));
                if out.len() >= 10 {
                    break;
                }
            }
        }
        out
    });

    let hero_entry = use_memo(move || {
        let lib = library.read();
        let conf = config.read();
        let track_by_id: HashMap<String, &Track> = lib
            .jellyfin_tracks
            .iter()
            .filter_map(|t| server_track_id(&t.path.to_string_lossy()).map(|id| (id, t)))
            .collect();
        let album_by_id: HashMap<&str, &Album> = lib
            .jellyfin_albums
            .iter()
            .map(|a| (a.id.as_str(), a))
            .collect();

        for id in conf.recently_played_server.iter() {
            if let Some(track) = track_by_id.get(id) {
                if is_unknown_artist(&track.artist) && track.title.trim().is_empty() {
                    continue;
                }
                let album = album_by_id.get(track.album_id.as_str()).copied().cloned();
                let cover = track_cover_url(&conf, track);
                return Some(((*track).clone(), album, cover));
            }
        }
        None
    });

    let made_for_you = use_memo(move || -> (String, Vec<AlbumCard>) {
        let lib = library.read();
        let conf = config.read();
        let mut genre_scores: HashMap<String, u64> = HashMap::new();
        let album_genre: HashMap<&str, &str> = lib
            .jellyfin_albums
            .iter()
            .map(|a| (a.id.as_str(), a.genre.as_str()))
            .collect();
        for track in &lib.jellyfin_tracks {
            let path = track.path.to_string_lossy().to_string();
            let plays = conf.listen_counts.get(&path).copied().unwrap_or(0);
            if plays == 0 {
                continue;
            }
            if let Some(genre) = album_genre.get(track.album_id.as_str()) {
                if genre.trim().is_empty() {
                    continue;
                }
                *genre_scores.entry(genre.to_string()).or_insert(0) += plays;
            }
        }
        let top_genre = genre_scores
            .into_iter()
            .max_by_key(|(_, v)| *v)
            .map(|(k, _)| k);
        let Some(top_genre) = top_genre else {
            return (String::new(), Vec::new());
        };
        let mut albums: Vec<Album> = lib
            .jellyfin_albums
            .iter()
            .filter(|a| {
                a.genre == top_genre && !is_unknown_album(&a.title) && !is_unknown_artist(&a.artist)
            })
            .cloned()
            .collect();
        let mut rng = thread_rng();
        albums.shuffle(&mut rng);
        albums.truncate(12);
        let cards = albums
            .into_iter()
            .map(|album| {
                let cover = album_cover_url(&conf, &album);
                (
                    album.id.clone(),
                    album.title.clone(),
                    album.artist.clone(),
                    cover,
                )
            })
            .collect();
        (top_genre, cards)
    });

    let jellyfin_artists = use_memo(move || {
        let lib = library.read();
        let conf = config.read();
        let mut unique_artists = std::collections::HashSet::new();
        let mut artist_list = Vec::new();
        let offline = *is_offline.read();
        for track in &lib.jellyfin_tracks {
            if offline {
                let s = track.path.to_string_lossy();
                let id = s.split(':').nth(1).unwrap_or(&s);
                let is_downloaded = if let Some(path_str) = conf.offline_tracks.get(id) {
                    std::path::Path::new(path_str).exists()
                } else {
                    false
                };
                if !is_downloaded {
                    continue;
                }
            }
            if is_unknown_artist(&track.artist) {
                continue;
            }
            if unique_artists.insert(track.artist.clone()) {
                let cover_url = track_cover_url(&conf, track);
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
        let conf = config.read();
        let offline = *is_offline.read();
        store
            .jellyfin_playlists
            .iter()
            .filter(|p| {
                if !offline {
                    return true;
                }
                !p.tracks.is_empty()
                    && p.tracks.iter().all(|tid| {
                        if let Some(path_str) = conf.offline_tracks.get(tid) {
                            std::path::Path::new(path_str).exists()
                        } else {
                            false
                        }
                    })
            })
            .rev()
            .take(10)
            .cloned()
            .map(|p| (p.id, p.name, p.tracks.len(), p.tracks.first().cloned()))
            .collect::<Vec<_>>()
    });

    let jellyfin_hero_cover = use_memo(move || {
        let conf = config.read();
        let lib = library.read();
        let shuffled = jellyfin_shuffled.read();
        let Some((album_id, ..)) = shuffled.first() else {
            return None;
        };
        let Some(album) = lib.jellyfin_albums.iter().find(|a| a.id == *album_id) else {
            return None;
        };
        let Some(server) = &conf.server else {
            return None;
        };
        album.cover_path.as_ref().and_then(|cover_path| {
            utils::jellyfin_image::jellyfin_image_url_from_path(
                &cover_path.to_string_lossy(),
                &server.url,
                server.access_token.as_deref(),
                1400,
                96,
            )
        })
    });

    let conf_snapshot = config.read();
    let is_modern = conf_snapshot.ui_style == UiStyle::Modern;
    let listen_now_style = conf_snapshot.listen_now_style;
    let sections: Vec<(String, bool)> = conf_snapshot
        .home_sections
        .iter()
        .map(|s| (s.key.clone(), s.enabled))
        .collect();
    drop(conf_snapshot);

    let scroll_container = move |id: &str, direction: i32| {
        let script = format!(
            "document.getElementById('{}').scrollBy({{ left: {}, behavior: 'smooth' }})",
            id,
            direction * 300
        );
        let _ = document::eval(&script);
    };

    let edit = *edit_mode.read();
    let total = sections.len();

    rsx! {
        div {
            for (idx, (key, enabled)) in sections.into_iter().enumerate() {
                {
                    let key_for_render = key.clone();
                    let key_toggle = key.clone();
                    let key_up = key.clone();
                    let key_down = key.clone();
                    if !enabled && !edit {
                        rsx! {}
                    } else {
                        rsx! {
                            div {
                                key: "{key}",
                                class: if !enabled { "opacity-40" } else { "" },
                                if edit {
                                    div { class: "flex items-center justify-between gap-2 mb-2 px-2 py-2 rounded-lg bg-white/5 border border-white/10",
                                        div { class: "flex items-center gap-2 text-white/80 text-xs font-bold uppercase tracking-wider",
                                            i { class: "fa-solid fa-grip-vertical text-white/30" }
                                            span { "{section_label(&key)}" }
                                        }
                                        div { class: "flex items-center gap-1",
                                            if key == "listen_now" {
                                                button {
                                                    class: "px-3 h-7 rounded-md bg-white/5 hover:bg-white/15 text-white/70 hover:text-white text-xs font-semibold transition-colors",
                                                    title: i18n::t("listen_now_layout").to_string(),
                                                    onclick: move |_| {
                                                        let mut conf = config.write();
                                                        conf.listen_now_style = match conf.listen_now_style {
                                                            ListenNowStyle::List => ListenNowStyle::Cards,
                                                            ListenNowStyle::Cards => ListenNowStyle::List,
                                                        };
                                                    },
                                                    i { class: if listen_now_style == ListenNowStyle::Cards { "fa-solid fa-grip-horizontal mr-1" } else { "fa-solid fa-list mr-1" } }
                                                    if listen_now_style == ListenNowStyle::Cards { {i18n::t("layout_cards").to_string()} } else { {i18n::t("layout_list").to_string()} }
                                                }
                                            }
                                            button {
                                                class: "w-7 h-7 rounded-md bg-white/5 hover:bg-white/15 text-white/70 hover:text-white transition-colors",
                                                title: i18n::t("move_up").to_string(),
                                                disabled: idx == 0,
                                                onclick: move |_| {
                                                    let mut conf = config.write();
                                                    if let Some(i) = conf.home_sections.iter().position(|s| s.key == key_up) {
                                                        if i > 0 { conf.home_sections.swap(i, i - 1); }
                                                    }
                                                },
                                                i { class: "fa-solid fa-chevron-up text-xs" }
                                            }
                                            button {
                                                class: "w-7 h-7 rounded-md bg-white/5 hover:bg-white/15 text-white/70 hover:text-white transition-colors",
                                                title: i18n::t("move_down").to_string(),
                                                disabled: idx + 1 >= total,
                                                onclick: move |_| {
                                                    let mut conf = config.write();
                                                    if let Some(i) = conf.home_sections.iter().position(|s| s.key == key_down) {
                                                        if i + 1 < conf.home_sections.len() { conf.home_sections.swap(i, i + 1); }
                                                    }
                                                },
                                                i { class: "fa-solid fa-chevron-down text-xs" }
                                            }
                                            button {
                                                class: if enabled {
                                                    "px-3 h-7 rounded-md bg-indigo-500/20 hover:bg-indigo-500/30 text-indigo-300 text-xs font-semibold transition-colors"
                                                } else {
                                                    "px-3 h-7 rounded-md bg-white/5 hover:bg-white/15 text-white/60 text-xs font-semibold transition-colors"
                                                },
                                                onclick: move |_| {
                                                    let mut conf = config.write();
                                                    if let Some(s) = conf.home_sections.iter_mut().find(|s| s.key == key_toggle) {
                                                        s.enabled = !s.enabled;
                                                    }
                                                },
                                                i { class: if enabled { "fa-solid fa-eye mr-1" } else { "fa-solid fa-eye-slash mr-1" } }
                                                if enabled { {i18n::t("hide_section").to_string()} } else { {i18n::t("show_section").to_string()} }
                                            }
                                        }
                                    }
                                }
                                {render_server_section(
                                    &key_for_render,
                                    library,
                                    favorites_store,
                                    config,
                                    edit,
                                    is_modern,
                                    listen_now_style,
                                    jellyfin_shuffled(),
                                    jellyfin_hero_cover(),
                                    continue_listening(),
                                    hero_entry(),
                                    jellyfin_artists(),
                                    new_releases(),
                                    made_for_you(),
                                    recently_added(),
                                    recent_playlists(),
                                    on_select_album,
                                    on_play_album,
                                    on_select_playlist,
                                    on_search_artist,
                                    scroll_container,
                                )}
                            }
                        }
                    }
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_server_section(
    key: &str,
    library: Signal<Library>,
    favorites_store: Signal<FavoritesStore>,
    config: Signal<AppConfig>,
    edit: bool,
    is_modern: bool,
    listen_now_style: ListenNowStyle,
    jellyfin_shuffled: Vec<AlbumCard>,
    hero_cover: Option<String>,
    continue_listening: Vec<(Track, Option<Album>, Option<String>)>,
    hero_entry: Option<(Track, Option<Album>, Option<String>)>,
    artists: Vec<(String, Option<String>)>,
    new_releases: Vec<AlbumCard>,
    made_for_you: (String, Vec<AlbumCard>),
    recently_added: Vec<AlbumCard>,
    recent_playlists: Vec<(String, String, usize, Option<String>)>,
    on_select_album: EventHandler<String>,
    on_play_album: EventHandler<String>,
    on_select_playlist: EventHandler<String>,
    on_search_artist: EventHandler<String>,
    scroll_container: impl Fn(&str, i32) + Copy + 'static,
) -> Element {
    match key {
        "hero" => rsx! {
            ServerHeroBanner {
                library,
                favorites_store,
                config,
                edit,
                is_modern,
                hero_entry,
                hero_cover,
                on_play_album,
            }
        },
        "continue_listening" => render_continue_listening(
            is_modern,
            continue_listening,
            on_select_album,
            on_play_album,
            scroll_container,
        ),
        "listen_now" => render_listen_now(
            is_modern,
            listen_now_style,
            jellyfin_shuffled,
            on_select_album,
            on_play_album,
        ),
        "top_artists" => render_top_artists(is_modern, artists, on_search_artist, scroll_container),
        "new_releases" => render_albums_row(
            "jelly-albums-scroll",
            i18n::t("new_releases").to_string(),
            i18n::t("albums").to_string(),
            is_modern,
            new_releases,
            on_select_album,
            on_play_album,
            scroll_container,
        ),
        "made_for_you" => {
            let (genre, albums) = made_for_you;
            let eyebrow = if genre.is_empty() {
                i18n::t("music").to_string()
            } else {
                genre
            };
            render_albums_row(
                "jelly-made-for-you-scroll",
                i18n::t("made_for_you").to_string(),
                eyebrow,
                is_modern,
                albums,
                on_select_album,
                on_play_album,
                scroll_container,
            )
        }
        "recently_added" => render_albums_row(
            "jelly-recently-added-scroll",
            i18n::t("recently_added").to_string(),
            i18n::t("library").to_string(),
            is_modern,
            recently_added,
            on_select_album,
            on_play_album,
            scroll_container,
        ),
        "playlists" => render_playlists(
            library,
            config,
            is_modern,
            recent_playlists,
            on_select_playlist,
            scroll_container,
        ),
        _ => rsx! {},
    }
}

#[component]
fn ServerHeroBanner(
    library: Signal<Library>,
    favorites_store: Signal<FavoritesStore>,
    mut config: Signal<AppConfig>,
    edit: bool,
    is_modern: bool,
    hero_entry: Option<(Track, Option<Album>, Option<String>)>,
    hero_cover: Option<String>,
    on_play_album: EventHandler<String>,
) -> Element {
    let mut is_resizing = use_signal(|| false);
    let mut start_y = use_signal(|| 0.0_f64);
    let mut start_h = use_signal(|| 0_u32);

    use_effect(move || {
        if *is_resizing.read() {
            let sy = *start_y.peek();
            let sh = *start_h.peek();
            spawn(async move {
                let mut eval = dioxus::document::eval(
                    r#"
                    const handleMouseMove = (e) => { dioxus.send(e.clientY); };
                    const handleMouseUp = () => {
                        dioxus.send("stop");
                        window.removeEventListener('mousemove', handleMouseMove);
                        window.removeEventListener('mouseup', handleMouseUp);
                    };
                    window.addEventListener('mousemove', handleMouseMove);
                    window.addEventListener('mouseup', handleMouseUp);
                    "#,
                );

                while let Ok(val) = eval.recv::<serde_json::Value>().await {
                    if let Some(y) = val.as_f64() {
                        let delta = y - sy;
                        let new_h = ((sh as f64) + delta).clamp(140.0, 800.0) as u32;
                        config.write().hero_height = new_h;
                    } else if val.as_str() == Some("stop") {
                        is_resizing.set(false);
                        break;
                    }
                }
            });
        }
    });

    let hero_height = config.read().hero_height;
    let section_class = if is_modern {
        "relative rounded-xl overflow-hidden mb-10"
    } else {
        "relative rounded-3xl overflow-hidden mb-12"
    };
    let section_style = format!("height: {hero_height}px;");

    let show_empty_state = hero_entry.is_none();
    let hero_title = hero_entry
        .as_ref()
        .map(|(track, album_opt, _)| {
            if let Some(album) = album_opt.as_ref() {
                if !is_unknown_album(&album.title) {
                    return album.title.clone();
                }
            }
            track.title.clone()
        })
        .unwrap_or_default();
    let hero_artist = hero_entry
        .as_ref()
        .map(|(track, album_opt, _)| {
            if !is_unknown_artist(&track.artist) {
                return track.artist.clone();
            }
            album_opt
                .as_ref()
                .map(|a| a.artist.clone())
                .unwrap_or_default()
        })
        .unwrap_or_default();

    rsx! {
        section { class: "{section_class}", style: "{section_style}",
            if !show_empty_state {
                if let Some((_, _album_opt, entry_cover)) = hero_entry.as_ref() {
                    div { class: "absolute inset-0",
                        if let Some(url) = entry_cover.clone().or(hero_cover.clone()) {
                            img { src: "{url}", class: "w-full h-full object-cover", decoding: "async" }
                        }
                        div { class: "absolute inset-0 bg-gradient-to-r from-black/90 via-black/40 to-transparent" }
                    }
                }
                div { class: "relative h-full flex flex-col justify-center p-8 md:p-12",
                    span { class: "text-indigo-400 font-bold tracking-widest uppercase text-[10px] mb-3 flex items-center gap-2",
                        i { class: "fa-solid fa-star text-[8px]" }
                        "{i18n::t(\"featured_album\")}"
                    }
                    h1 { class: "text-3xl md:text-5xl font-black text-white mb-4 leading-tight max-w-xl break-words", "{hero_title}" }
                    if !hero_artist.is_empty() {
                        p { class: "text-base md:text-lg text-white/60 mb-8 font-medium line-clamp-1 max-w-md", "{i18n::t_with(\"by_artist_full\", &[(\"artist\", hero_artist.clone())])}" }
                    }
                    div { class: "flex items-center gap-4",
                        button {
                            class: "flex items-center gap-3 bg-white text-black px-8 py-3 rounded-full font-bold hover:bg-white/90 hover:scale-105 active:scale-95 transition-all w-fit",
                            onclick: {
                                let id = hero_entry.as_ref().and_then(|(_, a, _)| a.as_ref().map(|a| a.id.clone()));
                                move |_| {
                                    if let Some(id) = id.clone() {
                                        on_play_album.call(id)
                                    }
                                }
                            },
                            i { class: "fa-solid fa-play text-[10px]" }
                            span { class: "text-sm", "{i18n::t(\"start_listening\")}" }
                        }
                        {
                            let album_id_hero = hero_entry.as_ref().and_then(|(_, a, _)| a.as_ref().map(|a| a.id.clone()));
                            let jelly_hero_fav = {
                                let lib = library.read();
                                let store = favorites_store.read();
                                let tracks: Vec<_> = lib.jellyfin_tracks.iter()
                                    .filter(|t| album_id_hero.as_deref() == Some(t.album_id.as_str()))
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
                            let mut favorites_store = favorites_store;
                            rsx! {
                                button {
                                    class: "{hero_heart_class}",
                                    onclick: move |_| {
                                        let tracks: Vec<_> = {
                                            let lib = library.read();
                                            lib.jellyfin_tracks.iter()
                                                .filter(|t| album_id_hero.as_deref() == Some(t.album_id.as_str()))
                                                .cloned()
                                                .collect()
                                        };
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

            if edit {
                div {
                    class: "absolute bottom-0 left-0 right-0 h-3 cursor-ns-resize flex items-center justify-center bg-black/40 hover:bg-indigo-500/40 transition-colors z-10",
                    title: "Drag to resize",
                    onmousedown: move |evt| {
                        evt.stop_propagation();
                        start_y.set(evt.client_coordinates().y);
                        start_h.set(config.peek().hero_height);
                        is_resizing.set(true);
                    },
                    div { class: "w-10 h-1 rounded-full bg-white/60" }
                }
            }
        }
    }
}

fn render_continue_listening(
    is_modern: bool,
    tracks: Vec<(Track, Option<Album>, Option<String>)>,
    on_select_album: EventHandler<String>,
    on_play_album: EventHandler<String>,
    scroll_container: impl Fn(&str, i32) + Copy + 'static,
) -> Element {
    if tracks.is_empty() {
        return rsx! { div {} };
    }
    rsx! {
        section { class: if is_modern { "mb-10" } else { "mb-12" },
            div { class: "flex items-center justify-between mb-6",
                div {
                    if is_modern {
                        p { class: "text-[10px] font-bold tracking-widest uppercase mb-0.5", style: "color: rgba(255,255,255,0.35);", "{i18n::t(\"library\")}" }
                    }
                    h2 { class: if is_modern { "text-2xl font-bold text-white" } else { "text-2xl font-bold text-white tracking-tight" }, "{i18n::t(\"continue_listening\")}" }
                }
                div { class: "flex gap-2",
                    button {
                        class: "w-8 h-8 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-all",
                        onclick: move |_| scroll_container("jelly-continue-scroll", -1),
                        i { class: "fa-solid fa-chevron-left text-sm" }
                    }
                    button {
                        class: "w-8 h-8 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-all",
                        onclick: move |_| scroll_container("jelly-continue-scroll", 1),
                        i { class: "fa-solid fa-chevron-right text-sm" }
                    }
                }
            }
            div {
                id: "jelly-continue-scroll",
                class: "flex overflow-x-auto gap-5 pb-6 pt-2 scrollbar-hide scroll-smooth -mx-2 px-2",
                for (track, album_opt, cover_url) in tracks {
                    {
                        let title = track.title.clone();
                        let artist = track.artist.clone();
                        let album_title = album_opt
                            .as_ref()
                            .map(|a| a.title.clone())
                            .unwrap_or_else(|| track.album.clone());
                        let album_id_opt = album_opt.as_ref().map(|a| a.id.clone());
                        let album_id_click = album_id_opt.clone();
                        let album_id_play = album_id_opt.clone();
                        let key = track.path.to_string_lossy().to_string();
                        rsx! {
                            div {
                                key: "{key}",
                                class: "flex-none w-44 group cursor-pointer",
                                onclick: move |_| {
                                    if let Some(id) = album_id_click.clone() {
                                        on_select_album.call(id);
                                    }
                                },
                                div { class: "aspect-square rounded-xl bg-stone-800 mb-3 overflow-hidden relative",
                                    if let Some(url) = cover_url {
                                        img { src: "{url}", class: "w-full h-full object-cover group-hover:scale-105 transition-transform duration-500", decoding: "async", loading: "lazy" }
                                    } else {
                                        div { class: "w-full h-full flex items-center justify-center",
                                            i { class: "fa-solid fa-music text-3xl text-white/20" }
                                        }
                                    }
                                    div {
                                        class: "absolute right-2 bottom-2 w-10 h-10 rounded-full flex items-center justify-center opacity-0 group-hover:opacity-100 transition-all translate-y-2 group-hover:translate-y-0",
                                        style: "background: var(--color-indigo-500);",
                                        onclick: move |evt| {
                                            evt.stop_propagation();
                                            if let Some(id) = album_id_play.clone() {
                                                on_play_album.call(id);
                                            }
                                        },
                                        i { class: "fa-solid fa-play text-white text-xs ml-0.5" }
                                    }
                                }
                                h3 { class: "text-white font-semibold truncate text-sm", "{title}" }
                                p { class: "text-xs truncate mt-0.5 text-white/50", "{artist} — {album_title}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn render_listen_now(
    is_modern: bool,
    listen_now_style: ListenNowStyle,
    jellyfin_shuffled: Vec<AlbumCard>,
    on_select_album: EventHandler<String>,
    on_play_album: EventHandler<String>,
) -> Element {
    if jellyfin_shuffled.is_empty() {
        return rsx! { div {} };
    }
    let use_cards = listen_now_style == ListenNowStyle::Cards;
    rsx! {
        section { class: if is_modern { "mb-10" } else { "mb-12" },
            div { class: "flex items-end justify-between mb-6",
                div {
                    if is_modern {
                        p { class: "text-[10px] font-bold tracking-widest uppercase mb-0.5", style: "color: rgba(255,255,255,0.35);", "{i18n::t(\"music\")}" }
                    }
                    h2 { class: if is_modern { "text-2xl font-bold text-white" } else { "text-3xl font-extrabold text-white tracking-tight leading-none" }, "{i18n::t(\"listen_now\")}" }
                }
            }
            if use_cards {
                div { class: "flex overflow-x-auto gap-4 pb-4 scrollbar-hide scroll-smooth -mx-2 px-2",
                    for (album_id, title, artist, cover_url) in jellyfin_shuffled.iter().skip(1).take(10).cloned() {
                        div {
                            class: "flex-none w-40 group cursor-pointer",
                            onclick: {
                                let id = album_id.clone();
                                move |_| on_select_album.call(id.clone())
                            },
                            div { class: "aspect-square rounded-lg bg-stone-800 mb-2 overflow-hidden relative",
                                if let Some(url) = cover_url {
                                    img { src: "{url}", class: "w-full h-full object-cover group-hover:scale-105 transition-transform duration-500", decoding: "async", loading: "lazy" }
                                } else {
                                    div { class: "w-full h-full flex items-center justify-center",
                                        i { class: "fa-solid fa-compact-disc text-2xl text-white/20" }
                                    }
                                }
                                div {
                                    class: "absolute right-2 bottom-2 w-9 h-9 rounded-full flex items-center justify-center opacity-0 group-hover:opacity-100 transition-all translate-y-2 group-hover:translate-y-0",
                                    style: "background: var(--color-indigo-500);",
                                    onclick: {
                                        let id = album_id.clone();
                                        move |evt| { evt.stop_propagation(); on_play_album.call(id.clone()); }
                                    },
                                    i { class: "fa-solid fa-play text-white text-xs ml-0.5" }
                                }
                            }
                            h3 { class: "text-white font-semibold truncate text-sm", "{title}" }
                            p { class: "text-xs truncate mt-0.5", style: "color: rgba(255,255,255,0.45);", "{artist}" }
                        }
                    }
                }
            } else {
                div { class: "grid grid-cols-[repeat(auto-fill,minmax(350px,1fr))] gap-4",
                    for (album_id, title, artist, cover_url) in jellyfin_shuffled.iter().skip(1).take(8).cloned() {
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
    }
}

fn render_top_artists(
    is_modern: bool,
    artists: Vec<(String, Option<String>)>,
    on_search_artist: EventHandler<String>,
    scroll_container: impl Fn(&str, i32) + Copy + 'static,
) -> Element {
    if artists.is_empty() {
        return rsx! { div {} };
    }
    rsx! {
        section { class: if is_modern { "mt-10" } else { "mt-12" },
            div { class: "flex items-center justify-between mb-6",
                div {
                    if is_modern {
                        p { class: "text-[10px] font-bold tracking-widest uppercase mb-0.5", style: "color: rgba(255,255,255,0.35);", "{i18n::t(\"artists\")}" }
                    }
                    h2 { class: if is_modern { "text-2xl font-bold text-white" } else { "text-2xl font-bold text-white tracking-tight" }, "{i18n::t(\"top_artists\")}" }
                }
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
                for (artist, cover_url) in artists {
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
}

fn render_albums_row(
    scroll_id: &'static str,
    title: String,
    eyebrow: String,
    is_modern: bool,
    albums: Vec<AlbumCard>,
    on_select_album: EventHandler<String>,
    on_play_album: EventHandler<String>,
    scroll_container: impl Fn(&str, i32) + Copy + 'static,
) -> Element {
    if albums.is_empty() {
        return rsx! { div {} };
    }
    rsx! {
        section { class: if is_modern { "mt-10" } else { "mt-12" },
            div { class: "flex items-center justify-between mb-6",
                div {
                    if is_modern {
                        p { class: "text-[10px] font-bold tracking-widest uppercase mb-0.5", style: "color: rgba(255,255,255,0.35);", "{eyebrow}" }
                    }
                    h2 { class: if is_modern { "text-2xl font-bold text-white" } else { "text-2xl font-bold text-white tracking-tight" }, "{title}" }
                }
                div { class: "flex gap-2",
                    button {
                        class: "w-8 h-8 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-all hover:scale-105",
                        onclick: move |_| scroll_container(scroll_id, -1),
                        i { class: "fa-solid fa-chevron-left text-sm" }
                    }
                    button {
                        class: "w-8 h-8 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-all hover:scale-105",
                        onclick: move |_| scroll_container(scroll_id, 1),
                        i { class: "fa-solid fa-chevron-right text-sm" }
                    }
                }
            }
            div {
                id: "{scroll_id}",
                class: "flex overflow-x-auto gap-5 pb-6 pt-2 overflow-y-visible scrollbar-hide scroll-smooth -mx-2 px-2",
                for (album_id, title, artist, cover_url) in albums {
                    div {
                        class: "flex-none w-36 md:w-48 group cursor-pointer",
                        onclick: {
                            let id = album_id.clone();
                            move |_| on_select_album.call(id.clone())
                        },
                        div { class: "aspect-square rounded-2xl bg-stone-800/80 mb-4 overflow-hidden transition-all duration-300 relative",
                            if let Some(url) = cover_url {
                                img { src: "{url}", class: "w-full h-full object-cover group-hover:scale-105 transition-transform duration-500", decoding: "async", loading: "lazy" }
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
}

fn render_playlists(
    library: Signal<Library>,
    config: Signal<AppConfig>,
    is_modern: bool,
    recent_playlists: Vec<(String, String, usize, Option<String>)>,
    on_select_playlist: EventHandler<String>,
    scroll_container: impl Fn(&str, i32) + Copy + 'static,
) -> Element {
    if recent_playlists.is_empty() {
        return rsx! { div {} };
    }
    rsx! {
        section { class: if is_modern { "mt-10" } else { "mt-16" },
            div { class: "flex items-center justify-between mb-6",
                div {
                    if is_modern {
                        p { class: "text-[10px] font-bold tracking-widest uppercase mb-0.5", style: "color: rgba(255,255,255,0.35);", "{i18n::t(\"library\")}" }
                    }
                    h2 { class: if is_modern { "text-2xl font-bold text-white" } else { "text-2xl font-bold text-white tracking-tight" }, "{i18n::t(\"playlists\")}" }
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
                for (id, name, track_count, first_track_id) in recent_playlists {
                    {
                        let cover_url = if let Some(tid) = first_track_id {
                            let lib = library.peek();
                            lib.jellyfin_tracks
                                .iter()
                                .find(|t| {
                                    let s = t.path.to_string_lossy();
                                    s.split(':').nth(1).map(|id| id == tid).unwrap_or(false)
                                })
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
                                key: "{id}",
                                class: "flex-none w-40 md:w-48 group cursor-pointer",
                                onclick: {
                                    let id = id.clone();
                                    move |_| on_select_playlist.call(id.clone())
                                },
                                div { class: "aspect-square rounded-2xl bg-white/5 mb-4 overflow-hidden transition-all duration-500 relative",
                                    if let Some(url) = cover_url {
                                        img { src: "{url}", class: "w-full h-full object-cover group-hover:scale-110 transition-transform duration-700", decoding: "async", loading: "lazy" }
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
                                        {
                                            let track_text = i18n::t_with("music_playlist_count", &[("count", track_count.to_string())]);
                                            rsx! { "{track_text}" }
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
