use config::{AppConfig, ListenNowStyle, UiStyle};
use dioxus::prelude::*;
use hooks::use_db_queries::{
    use_active_source, use_album_tracks, use_albums, use_artist_sample_tracks, use_favorites,
    use_playlists, use_top_genre, use_tracks_by_keys,
};
use rand::rng;
use rand::seq::SliceRandom;
use reader::{Album, Track};
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

fn album_cover_url(conf: &AppConfig, album: &Album) -> Option<String> {
    ::server::cover::from_path(conf, album.cover_path.as_deref(), 384).map(|c| c.to_string())
}

/// A track's cover, source-agnostic via the cover seam — the track self-describes
/// its cover (a local row's path is projected from its album by the DB read layer).
fn track_cover_url(conf: &AppConfig, track: &Track) -> Option<String> {
    ::server::cover::track(conf, track, 384).map(|c| c.to_string())
}

/// The source-agnostic Home body (sections + hero). Rendered for local and any
/// server; the active source decides the data, covers (via the source seam), the
/// recently-played list, and offline/sync gating.
#[component]
pub fn HomeBody(
    edit_mode: Signal<bool>,
    on_select_album: EventHandler<String>,
    on_play_album: EventHandler<String>,
    on_select_playlist: EventHandler<String>,
    on_search_artist: EventHandler<String>,
) -> Element {
    let is_offline = use_context::<Signal<bool>>();
    let mut config = use_context::<Signal<AppConfig>>();
    let source = use_active_source();
    let active_source = use_context::<Signal<::server::source::ActiveSource>>();
    let caps = use_memo(move || active_source.read().capabilities());
    let mut has_fetched = use_signal(|| false);

    let albums_res = use_albums(source);
    let playlists_res = use_playlists();
    // The artist-image caches the Top Artists row resolves through (read-only:
    // home triggers no photo fetch; the Artists page's pipeline fills these).
    let artist_images_res = hooks::use_db_queries::use_artist_images();
    let fetched_artist_images = use_context::<Signal<::server::cover::FetchedArtistImages>>();
    let offline_keys = use_memo(move || -> Vec<String> {
        if !(caps().downloads && *is_offline.read()) {
            return Vec::new();
        }
        config
            .read()
            .offline_tracks
            .iter()
            .filter(|(_, path)| std::path::Path::new(path).exists())
            .map(|(id, _)| id.clone())
            .collect()
    });
    let offline_tracks_res = use_tracks_by_keys(source, offline_keys);
    // Recently-played for the active source (each source keeps its own history).
    let recent_tracks_res = hooks::use_db_queries::use_recently_played(source);
    let top_genre_res = use_top_genre(source);
    let artist_samples_res = use_artist_sample_tracks(source, 30);

    // Servers fill an empty cache by syncing; local is populated by the scan.
    let mut fetch_remote = move || {
        has_fetched.set(true);
        spawn(async move {
            let _ = crate::server::subsonic_sync::sync_server_library(false).await;
        });
    };

    use_effect(move || {
        if !caps().sync || *has_fetched.read() {
            return;
        }
        if let Some(albums) = albums_res.read().as_ref() {
            if albums.is_empty() {
                fetch_remote();
            } else {
                has_fetched.set(true);
            }
        }
    });

    let jellyfin_albums_all = use_memo(move || -> Vec<AlbumCard> {
        let conf = config.read();

        let mut albums = albums_res.read().clone().unwrap_or_default();
        albums.sort_by(|a, b| {
            a.title
                .trim()
                .to_lowercase()
                .cmp(&b.title.trim().to_lowercase())
        });

        let mut unique_albums = Vec::new();
        let mut seen_titles = std::collections::HashSet::new();

        let offline = caps().downloads && *is_offline.read();
        let downloaded_album_ids: std::collections::HashSet<String> = if offline {
            offline_tracks_res
                .read()
                .clone()
                .unwrap_or_default()
                .iter()
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
        let mut rng = rng();
        let mut shuffled = albums.clone();
        shuffled.shuffle(&mut rng);
        shuffled
    });

    let new_releases = use_memo(move || -> Vec<AlbumCard> {
        let conf = config.read();
        let mut albums = albums_res.read().clone().unwrap_or_default();
        albums.sort_by_key(|b| std::cmp::Reverse(b.year));
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
        let conf = config.read();
        let all_albums = albums_res.read().clone().unwrap_or_default();
        let mut unique = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for album in all_albums.iter().rev() {
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
        let conf = config.read();
        let recent_tracks = recent_tracks_res.read().clone().unwrap_or_default();
        let all_albums = albums_res.read().clone().unwrap_or_default();
        let album_by_id: HashMap<&str, &Album> =
            all_albums.iter().map(|a| (a.id.as_str(), a)).collect();
        let mut out: Vec<(Track, Option<Album>, Option<String>)> = Vec::new();
        let mut seen_albums = std::collections::HashSet::new();
        for track in recent_tracks.iter() {
            if track.title.trim().is_empty() {
                continue;
            }
            let album = album_by_id.get(track.album_id.as_str()).copied().cloned();
            if let Some(ref album_ref) = album {
                if is_unknown_album(&album_ref.title) || is_unknown_artist(&album_ref.artist) {
                    continue;
                }
            } else if is_unknown_artist(&track.artist) {
                continue;
            }
            if let Some(ref a) = album
                && !seen_albums.insert(a.id.clone())
            {
                continue;
            }
            let cover = track_cover_url(&conf, track);
            out.push((track.clone(), album, cover));
            if out.len() >= 10 {
                break;
            }
        }
        out
    });

    let hero_entry = use_memo(move || {
        let conf = config.read();
        let recent_tracks = recent_tracks_res.read().clone().unwrap_or_default();
        let all_albums = albums_res.read().clone().unwrap_or_default();
        let album_by_id: HashMap<&str, &Album> =
            all_albums.iter().map(|a| (a.id.as_str(), a)).collect();

        for track in recent_tracks.iter() {
            if track.title.trim().is_empty() {
                continue;
            }
            let album = album_by_id.get(track.album_id.as_str()).copied().cloned();
            let cover = track_cover_url(&conf, track);
            return Some((track.clone(), album, cover));
        }
        None
    });

    let made_for_you = use_memo(move || -> (String, Vec<AlbumCard>) {
        let conf = config.read();
        let all_albums = albums_res.read().clone().unwrap_or_default();
        let Some(top_genre) = top_genre_res.read().clone().flatten() else {
            return (String::new(), Vec::new());
        };
        let mut albums: Vec<Album> = all_albums
            .iter()
            .filter(|a| {
                a.genre == top_genre && !is_unknown_album(&a.title) && !is_unknown_artist(&a.artist)
            })
            .cloned()
            .collect();
        let mut rng = rng();
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
        let conf = config.read();
        let albums = albums_res.read().clone().unwrap_or_default();
        let images = artist_images_res.read().clone().unwrap_or_default();
        let fetched = fetched_artist_images.read();
        let tracks = if caps().downloads && *is_offline.read() {
            let mut downloaded = offline_tracks_res.read().clone().unwrap_or_default();
            downloaded.sort_by_key(|a| a.artist.to_lowercase());
            downloaded
        } else {
            artist_samples_res.read().clone().unwrap_or_default()
        };
        let mut unique_artists = std::collections::HashSet::new();
        let mut artist_list = Vec::new();
        for track in &tracks {
            if is_unknown_artist(&track.artist) {
                continue;
            }
            if unique_artists.insert(track.artist.clone()) {
                // The same image chain the Artists grid uses: photo where one
                // exists, the track's album cover as the Library last resort
                // (a Remote catalog resolves photo-or-placeholder instead).
                let norm = utils::artist::normalize_artist_key(&track.artist);
                let album_cover = albums
                    .iter()
                    .find(|a| a.id == track.album_id)
                    .and_then(|a| a.cover_path.as_deref());
                let art = ::server::cover::ArtistArt::from_caches(
                    &images,
                    &fetched,
                    &norm,
                    &track.artist,
                    album_cover,
                    caps().artist_view,
                );
                let cover_url = ::server::cover::artist(&conf, art, 384).map(|c| c.to_string());
                artist_list.push((track.artist.clone(), cover_url));
            }
            if artist_list.len() >= 10 {
                break;
            }
        }
        artist_list
    });

    let playlist_cover_keys = use_memo(move || -> Vec<String> {
        let store = playlists_res.read().clone().unwrap_or_default();
        store
            .playlists
            .iter()
            .filter_map(|p| p.tracks.first().cloned())
            .collect()
    });
    let playlist_cover_tracks_res = use_tracks_by_keys(source, playlist_cover_keys);

    let recent_playlists = use_memo(move || {
        let store = playlists_res.read().clone().unwrap_or_default();
        let cover_tracks = playlist_cover_tracks_res.read().clone().unwrap_or_default();
        let conf = config.read();
        let offline = caps().downloads && *is_offline.read();
        store
            .playlists
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
            .map(|p| {
                let cover_url = p.tracks.first().and_then(|tid| {
                    cover_tracks
                        .iter()
                        .find(|t| {
                            let id = t.id.key();
                            !id.is_empty() && id.as_ref() == tid.as_str()
                        })
                        .and_then(|t| track_cover_url(&conf, t))
                });
                (p.id, p.name, p.tracks.len(), cover_url)
            })
            .collect::<Vec<_>>()
    });

    let jellyfin_hero_cover = use_memo(move || {
        let conf = config.read();
        let entry = hero_entry.read();
        let (_, album_opt, _) = entry.as_ref()?;
        let album = album_opt.as_ref()?;
        ::server::cover::from_path(&conf, album.cover_path.as_deref(), 1400).map(|c| c.to_string())
    });

    let conf_snapshot = config.read();
    let is_vaxry = conf_snapshot.ui_style == UiStyle::Vaxry;
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
                                        div { class: "flex items-center gap-2 text-white/80 text-xs font-bold",
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
                                                    if let Some(i) = conf.home_sections.iter().position(|s| s.key == key_up)
                                                        && i > 0 { conf.home_sections.swap(i, i - 1); }
                                                },
                                                i { class: "fa-solid fa-chevron-up text-xs" }
                                            }
                                            button {
                                                class: "w-7 h-7 rounded-md bg-white/5 hover:bg-white/15 text-white/70 hover:text-white transition-colors",
                                                title: i18n::t("move_down").to_string(),
                                                disabled: idx + 1 >= total,
                                                onclick: move |_| {
                                                    let mut conf = config.write();
                                                    if let Some(i) = conf.home_sections.iter().position(|s| s.key == key_down)
                                                        && i + 1 < conf.home_sections.len() { conf.home_sections.swap(i, i + 1); }
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
                                    config,
                                    edit,
                                    is_vaxry,
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
    config: Signal<AppConfig>,
    edit: bool,
    is_vaxry: bool,
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
                config,
                edit,
                is_vaxry,
                hero_entry,
                hero_cover,
                on_play_album,
            }
        },
        "continue_listening" => render_continue_listening(
            is_vaxry,
            continue_listening,
            on_select_album,
            on_play_album,
            scroll_container,
        ),
        "listen_now" => render_listen_now(
            is_vaxry,
            listen_now_style,
            jellyfin_shuffled,
            on_select_album,
            on_play_album,
        ),
        "top_artists" => render_top_artists(is_vaxry, artists, on_search_artist, scroll_container),
        "new_releases" => render_albums_row(
            "jelly-albums-scroll",
            i18n::t("new_releases").to_string(),
            i18n::t("albums").to_string(),
            is_vaxry,
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
                is_vaxry,
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
            is_vaxry,
            recently_added,
            on_select_album,
            on_play_album,
            scroll_container,
        ),
        "playlists" => render_playlists(
            config,
            is_vaxry,
            recent_playlists,
            on_select_playlist,
            scroll_container,
        ),
        _ => rsx! {},
    }
}

#[component]
fn ServerHeroBanner(
    mut config: Signal<AppConfig>,
    edit: bool,
    is_vaxry: bool,
    hero_entry: Option<(Track, Option<Album>, Option<String>)>,
    hero_cover: Option<String>,
    on_play_album: EventHandler<String>,
) -> Element {
    let mut is_resizing = use_signal(|| false);
    let mut start_y = use_signal(|| 0.0_f64);
    let mut start_h = use_signal(|| 0_u32);

    let source = use_active_source();
    // The track's own `album_id` (not the resolved `Album`, which lags behind a
    // separate albums query) — so the play button and the favorite-state heart
    // work the instant the hero track renders, not only once albums load.
    let hero_album_id_val = hero_entry
        .as_ref()
        .map(|(t, _, _)| t.album_id.clone())
        .unwrap_or_default();
    let mut hero_album_id = use_signal(|| hero_album_id_val.clone());
    if *hero_album_id.peek() != hero_album_id_val {
        hero_album_id.set(hero_album_id_val);
    }
    let hero_album_id_memo = use_memo(move || hero_album_id.read().clone());
    let hero_tracks_res = use_album_tracks(source, hero_album_id_memo);
    let favorites_res = use_favorites();

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
    let section_class = if is_vaxry {
        "relative rounded-xl overflow-hidden mb-10"
    } else {
        "relative rounded-xl overflow-hidden mb-12"
    };
    let section_style = format!("height: {hero_height}px;");

    let show_empty_state = hero_entry.is_none();
    let hero_title = hero_entry
        .as_ref()
        .map(|(track, _, _)| track.title.clone())
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
                    div { class: "absolute inset-0 overflow-hidden",
                        if let Some(url) = hero_cover.clone().or(entry_cover.clone()) {
                            {
                                let src = if url.starts_with("artwork://") {
                                    format!("{url}&hq=1")
                                } else {
                                    url.clone()
                                };
                                rsx! {
                                    img {
                                        src: "{src}",
                                        class: "absolute inset-0 w-full h-full object-cover object-center",
                                        decoding: "async",
                                    }
                                }
                            }
                        }
                        div { class: "absolute inset-0 bg-gradient-to-r from-black/95 via-black/60 to-black/20" }
                        div { class: "absolute inset-0 bg-gradient-to-t from-black/50 to-transparent" }
                    }
                }
                div { class: "relative h-full flex flex-col justify-center p-8 md:p-12",
                    span { class: "text-indigo-400 font-bold text-[10px] mb-3 flex items-center gap-2",
                        i { class: "fa-solid fa-star text-[8px]" }
                        "{i18n::t(\"featured_album\")}"
                    }
                    h1 { class: "text-3xl md:text-5xl font-semibold tracking-tight text-white mb-4 leading-tight break-words", style: "overflow: hidden; text-overflow:ellipsis;white-space: nowrap;", "{hero_title}" }
                    if !hero_artist.is_empty() {
                        p { class: "text-base md:text-lg text-white/60 mb-8 font-medium line-clamp-1 max-w-md", "{i18n::t_with(\"by_artist_full\", &[(\"artist\", hero_artist.clone())])}" }
                    }
                    div { class: "flex items-center gap-4",
                        button {
                            class: "flex items-center gap-3 bg-white text-black px-8 py-3 rounded-full font-bold hover:bg-white/90 hover:scale-105 active:scale-95 transition-all w-fit",
                            onclick: {
                                let id = hero_entry.as_ref().map(|(t, _, _)| t.album_id.clone());
                                move |_| {
                                    if let Some(id) = id.clone().filter(|s| !s.is_empty()) {
                                        on_play_album.call(id)
                                    }
                                }
                            },
                            i { class: "fa-solid fa-play text-[10px]" }
                            span { class: "text-sm", "{i18n::t(\"start_listening\")}" }
                        }
                        {
                            let jelly_hero_fav = {
                                let tracks = if hero_album_id.read().is_empty() {
                                    Vec::new()
                                } else {
                                    hero_tracks_res.read().clone().unwrap_or_default()
                                };
                                let favs: std::collections::HashSet<String> = favorites_res
                                    .read()
                                    .clone()
                                    .unwrap_or_default()
                                    .into_iter()
                                    .collect();
                                !tracks.is_empty() && tracks.iter().all(|t| {
                                    let id = t.id.key();
                                    !id.is_empty() && favs.contains(id.as_ref())
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
                                        let tracks: Vec<_> = if hero_album_id.peek().is_empty() {
                                            Vec::new()
                                        } else {
                                            hero_tracks_res.read().clone().unwrap_or_default()
                                        };
                                        hooks::favorites::set_favorite_many(tracks, !jelly_hero_fav);
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
    is_vaxry: bool,
    tracks: Vec<(Track, Option<Album>, Option<String>)>,
    on_select_album: EventHandler<String>,
    on_play_album: EventHandler<String>,
    scroll_container: impl Fn(&str, i32) + Copy + 'static,
) -> Element {
    if tracks.is_empty() {
        return rsx! { div {} };
    }
    rsx! {
        section { class: if is_vaxry { "mb-10" } else { "mb-12" },
            div { class: "flex items-center justify-between mb-6",
                div {
                    if is_vaxry {
                        p { class: "text-[10px] font-bold mb-0.5", style: "color: rgba(255,255,255,0.35);", "{i18n::t(\"library\")}" }
                    }
                    h2 { class: "text-2xl font-semibold tracking-tight text-white", "{i18n::t(\"continue_listening\")}" }
                }
                div { class: "flex gap-2",
                    button {
                        class: "w-9 h-9 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-colors active:scale-95",
                        onclick: move |_| scroll_container("jelly-continue-scroll", -1),
                        i { class: "fa-solid fa-chevron-left text-sm" }
                    }
                    button {
                        class: "w-9 h-9 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-colors active:scale-95",
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
                        let key = track.id.uid();
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
                                    components::album_play_button::AlbumPlayButton {
                                        album_id: album_id_play.clone(),
                                        on_play_album,
                                        class: "absolute right-2 bottom-2 w-10 h-10 rounded-full flex items-center justify-center opacity-0 group-hover:opacity-100 transition-all translate-y-2 group-hover:translate-y-0".to_string(),
                                        style: "background: var(--color-indigo-500);".to_string(),
                                        icon_extra: "text-white text-xs".to_string(),
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
    is_vaxry: bool,
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
        section { class: if is_vaxry { "mb-10" } else { "mb-12" },
            div { class: "flex items-end justify-between mb-6",
                div {
                    if is_vaxry {
                        p { class: "text-[10px] font-bold mb-0.5", style: "color: rgba(255,255,255,0.35);", "{i18n::t(\"music\")}" }
                    }
                    h2 { class: if is_vaxry { "text-2xl font-semibold tracking-tight text-white" } else { "text-3xl font-semibold tracking-tight text-white leading-none" }, "{i18n::t(\"listen_now\")}" }
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
                            div { class: "aspect-square rounded-xl bg-stone-800 mb-2 overflow-hidden relative",
                                if let Some(url) = cover_url {
                                    img { src: "{url}", class: "w-full h-full object-cover group-hover:scale-105 transition-transform duration-500", decoding: "async", loading: "lazy" }
                                } else {
                                    div { class: "w-full h-full flex items-center justify-center",
                                        i { class: "fa-solid fa-compact-disc text-2xl text-white/20" }
                                    }
                                }
                                components::album_play_button::AlbumPlayButton {
                                    album_id: Some(album_id.clone()),
                                    on_play_album,
                                    class: "absolute right-2 bottom-2 w-9 h-9 rounded-full flex items-center justify-center opacity-0 group-hover:opacity-100 transition-all translate-y-2 group-hover:translate-y-0".to_string(),
                                    style: "background: var(--color-indigo-500);".to_string(),
                                    icon_extra: "text-white text-xs".to_string(),
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
                            class: "flex items-center bg-white/5 hover:bg-white/10 border border-white/5 rounded-xl cursor-pointer transition-all duration-300 group overflow-hidden pr-4",
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
                                components::album_play_button::AlbumPlayButton {
                                    album_id: Some(album_id.clone()),
                                    on_play_album,
                                    class: "w-8 h-8 rounded-full bg-white/10 flex items-center justify-center hover:bg-white/20 transition-colors".to_string(),
                                    icon_extra: "text-white/80 text-xs".to_string(),
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
    is_vaxry: bool,
    artists: Vec<(String, Option<String>)>,
    on_search_artist: EventHandler<String>,
    scroll_container: impl Fn(&str, i32) + Copy + 'static,
) -> Element {
    if artists.is_empty() {
        return rsx! { div {} };
    }
    rsx! {
        section { class: if is_vaxry { "mt-10" } else { "mt-12" },
            div { class: "flex items-center justify-between mb-6",
                div {
                    if is_vaxry {
                        p { class: "text-[10px] font-bold mb-0.5", style: "color: rgba(255,255,255,0.35);", "{i18n::t(\"artists\")}" }
                    }
                    h2 { class: "text-2xl font-semibold tracking-tight text-white", "{i18n::t(\"top_artists\")}" }
                }
                div { class: "flex gap-2",
                    button {
                        class: "w-9 h-9 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-colors active:scale-95",
                        onclick: move |_| scroll_container("jelly-artists-scroll", -1),
                        i { class: "fa-solid fa-chevron-left text-sm" }
                    }
                    button {
                        class: "w-9 h-9 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-colors active:scale-95",
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
    is_vaxry: bool,
    albums: Vec<AlbumCard>,
    on_select_album: EventHandler<String>,
    on_play_album: EventHandler<String>,
    scroll_container: impl Fn(&str, i32) + Copy + 'static,
) -> Element {
    if albums.is_empty() {
        return rsx! { div {} };
    }
    rsx! {
        section { class: if is_vaxry { "mt-10" } else { "mt-12" },
            div { class: "flex items-center justify-between mb-6",
                div {
                    if is_vaxry {
                        p { class: "text-[10px] font-bold mb-0.5", style: "color: rgba(255,255,255,0.35);", "{eyebrow}" }
                    }
                    h2 { class: "text-2xl font-semibold tracking-tight text-white", "{title}" }
                }
                div { class: "flex gap-2",
                    button {
                        class: "w-9 h-9 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-colors active:scale-95",
                        onclick: move |_| scroll_container(scroll_id, -1),
                        i { class: "fa-solid fa-chevron-left text-sm" }
                    }
                    button {
                        class: "w-9 h-9 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-colors active:scale-95",
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
                        div { class: "aspect-square rounded-xl bg-stone-800/80 mb-4 overflow-hidden transition-all duration-300 relative",
                            if let Some(url) = cover_url {
                                img { src: "{url}", class: "w-full h-full object-cover group-hover:scale-105 transition-transform duration-500", decoding: "async", loading: "lazy" }
                            } else {
                                div { class: "w-full h-full flex items-center justify-center border border-white/5 rounded-lg",
                                    i { class: "fa-solid fa-compact-disc text-4xl text-white/20" }
                                }
                            }
                            div { class: "absolute inset-0 bg-black/0 group-hover:bg-black/20 transition-colors duration-300" }
                            components::album_play_button::AlbumPlayButton {
                                album_id: Some(album_id.clone()),
                                on_play_album,
                                class: "absolute right-3 bottom-3 w-10 h-10 bg-white text-black rounded-full flex items-center justify-center translate-y-4 opacity-0 group-hover:translate-y-0 group-hover:opacity-100 transition-all duration-300".to_string(),
                                icon_extra: "text-sm".to_string(),
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
    _config: Signal<AppConfig>,
    is_vaxry: bool,
    recent_playlists: Vec<(String, String, usize, Option<String>)>,
    on_select_playlist: EventHandler<String>,
    scroll_container: impl Fn(&str, i32) + Copy + 'static,
) -> Element {
    if recent_playlists.is_empty() {
        return rsx! { div {} };
    }
    rsx! {
        section { class: if is_vaxry { "mt-10" } else { "mt-16" },
            div { class: "flex items-center justify-between mb-6",
                div {
                    if is_vaxry {
                        p { class: "text-[10px] font-bold mb-0.5", style: "color: rgba(255,255,255,0.35);", "{i18n::t(\"library\")}" }
                    }
                    h2 { class: "text-2xl font-semibold tracking-tight text-white", "{i18n::t(\"playlists\")}" }
                }
                div { class: "flex gap-2",
                    button {
                        class: "w-9 h-9 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-colors active:scale-95",
                        onclick: move |_| scroll_container("jelly-playlists-scroll", -1),
                        i { class: "fa-solid fa-chevron-left text-sm" }
                    }
                    button {
                        class: "w-9 h-9 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-colors active:scale-95",
                        onclick: move |_| scroll_container("jelly-playlists-scroll", 1),
                        i { class: "fa-solid fa-chevron-right text-sm" }
                    }
                }
            }
            div {
                id: "jelly-playlists-scroll",
                class: "flex overflow-x-auto gap-6 pb-6 pt-2 scrollbar-hide scroll-smooth -mx-2 px-2",
                for (id, name, track_count, cover_url) in recent_playlists {
                    {
                        rsx! {
                            div {
                                key: "{id}",
                                class: "flex-none w-40 md:w-48 group cursor-pointer",
                                onclick: {
                                    let id = id.clone();
                                    move |_| on_select_playlist.call(id.clone())
                                },
                                div { class: "aspect-square rounded-xl bg-white/5 mb-4 overflow-hidden transition-all duration-500 relative",
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
