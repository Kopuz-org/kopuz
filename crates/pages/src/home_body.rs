use components::dots_menu::{DotsMenu, MenuAction};
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

/// The hero stretches one cover across the full content width (up to 800px
/// tall), so it asks for far more pixels than the 384px grid cards.
const HERO_COVER_WIDTH: u32 = 1400;

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
    let caps = use_context::<Signal<api::SourceCapabilities>>();
    let mut has_fetched = use_signal(|| false);
    // Which card has its overflow menu open, keyed by track uid / playlist id.
    // Owned here because the section renderers are plain functions, so they
    // cannot hold hook state of their own.
    let active_card_menu = use_signal(|| None::<String>);

    let albums_res = use_albums(source);
    let playlists_res = use_playlists();
    // The artist-image caches the Top Artists row resolves through (read-only:
    // home triggers no photo fetch; the Artists page's pipeline fills these).
    let artist_images_res = hooks::use_db_queries::use_artist_images();
    let fetched_artist_images = use_context::<Signal<::server::cover::FetchedArtistImages>>();
    let downloaded_tracks = use_context::<hooks::downloads::DownloadedTracks>();
    let offline_keys = use_memo(move || -> Vec<String> {
        if !(caps().downloads && *is_offline.read()) {
            return Vec::new();
        }
        downloaded_tracks.0.read().iter().cloned().collect()
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
            let _ = crate::server::library_sync::sync_server_library().await;
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
                    caps().artists == api::ArtistPresentation::Library,
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
        let downloaded = downloaded_tracks.0.read();
        let offline = caps().downloads && *is_offline.read();
        store
            .playlists
            .iter()
            .filter(|p| {
                if !offline {
                    return true;
                }
                !p.tracks.is_empty() && p.tracks.iter().all(|tid| downloaded.contains(tid))
            })
            .rev()
            .take(10)
            .cloned()
            .map(|p| {
                let cover_url = {
                    if let Some(url) = ::server::cover::playlist(
                        &conf,
                        &p.id,
                        p.cover_path.as_deref(),
                        p.image_tag.as_deref(),
                        384,
                    ) {
                        Some(url.to_string())
                    } else {
                        p.tracks.first().and_then(|tid| {
                            cover_tracks
                                .iter()
                                .find(|t| {
                                    let id = t.id.key();
                                    !id.is_empty() && id.as_ref() == tid.as_str()
                                })
                                .and_then(|t| track_cover_url(&conf, t))
                        })
                    }
                };
                (p.id, p.name, p.tracks.len(), cover_url)
            })
            .collect::<Vec<_>>()
    });

    let hero_cover = use_memo(move || {
        let conf = config.read();
        let entry = hero_entry.read();
        let (track, album_opt, _) = entry.as_ref()?;
        // The album's own art first, but fall back to the track's — the albums
        // query lags the recently-played one, and not every album has a cover
        // path, which otherwise left the hero on the 384px card thumbnail.
        let cover = album_opt
            .as_ref()
            .and_then(|album| {
                ::server::cover::from_path(&conf, album.cover_path.as_deref(), HERO_COVER_WIDTH)
            })
            .or_else(|| ::server::cover::track(&conf, track, HERO_COVER_WIDTH))?;
        Some(components::high_quality_artwork_url(cover.to_string()))
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
                                    hero_cover(),
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
                                    active_card_menu,
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

#[path = "home_body_sections.rs"]
mod sections;
use sections::*;
