use api::{AlbumInfo as Album, TrackInfo as Track};
use components::dots_menu::{DotsMenu, MenuAction};
use config::{ListenNowStyle, Settings, UiStyle};
use dioxus::prelude::*;
use hooks::use_db_queries::{
    use_active_source, use_album_tracks, use_albums, use_albums_window, use_artist_sample_tracks,
    use_artists, use_downloaded_tracks, use_favorites, use_playlists, use_top_genre,
};
use rand::rng;
use rand::seq::SliceRandom;
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

/// The album a recently played track features in the hero, if it is filed under a known one.
fn featured_album(
    track: &Track,
    album_by_id: &HashMap<&str, &Album>,
) -> Option<(Track, Option<Album>, Option<String>)> {
    if track.album_id.trim().is_empty() {
        return None;
    }
    let album = album_by_id.get(track.album_id.as_str()).copied().cloned();
    let title = album
        .as_ref()
        .map_or(track.album.as_str(), |a| a.title.as_str());
    if is_unknown_album(title) {
        return None;
    }
    let cover = track_cover_url(track);
    Some((track.clone(), album, cover))
}

/// The hero's headline: the featured album's title and its billed artist.
fn featured_album_billing(track: &Track, album: Option<&Album>) -> (String, String) {
    let title = album.map_or(track.album.clone(), |a| a.title.clone());
    let artist = album
        .map(|a| a.artist.clone())
        .filter(|artist| !is_unknown_artist(artist))
        .unwrap_or_else(|| track.artist.clone());
    (title, artist)
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

fn album_cover_url(album: &Album) -> Option<String> {
    hooks::artwork::for_album(album, hooks::artwork::Size::Thumb).map(|cover| cover.to_string())
}

fn is_known(album: &Album) -> bool {
    !is_unknown_album(&album.title) && !is_unknown_artist(&album.artist)
}

fn album_card(album: &Album) -> AlbumCard {
    (
        album.id.clone(),
        album.title.clone(),
        album.artist.clone(),
        album_cover_url(album),
    )
}

/// A shelf's cards: the first known albums of a listing, in its order.
fn shelf(albums: &[Album]) -> Vec<AlbumCard> {
    albums
        .iter()
        .filter(|album| is_known(album))
        .take(SHELF_LEN)
        .map(album_card)
        .collect()
}

/// A track's cover: the row carries its own reference, so a mixed-source list
/// resolves without asking which service it came from.
fn track_cover_url(track: &Track) -> Option<String> {
    hooks::artwork::for_track(track, hooks::artwork::Size::Thumb).map(|cover| cover.to_string())
}

/// How many albums a shelf query pulls: a shelf shows 12, and dropping the unknown ones still has to fill it.
const SHELF_WINDOW: u32 = 64;

const SHELF_LEN: usize = 12;

/// The source-agnostic Home body (sections + hero). Rendered for any
/// source; the active source decides the data, covers (via the source seam), the
/// recently-played list, and offline/sync gating.
#[component]
pub fn HomeBody(
    edit_mode: Signal<bool>,
    on_select_album: EventHandler<String>,
    on_play_album: EventHandler<String>,
    on_select_playlist: EventHandler<String>,
    on_open_artist: EventHandler<String>,
) -> Element {
    let is_offline = use_context::<Signal<bool>>();
    let settings = use_context::<Signal<Settings>>();
    let mut prefs = hooks::ui_prefs::use_ui_prefs();
    let source = use_active_source();
    let caps = hooks::sources::use_capabilities();
    // Which card has its overflow menu open, keyed by track uid / playlist id.
    // Owned here because the section renderers are plain functions, so they
    // cannot hold hook state of their own.
    let active_card_menu = use_signal(|| None::<String>);

    // Offline every shelf holds only what has a downloaded copy.
    let offline = use_memo(move || caps().downloads && *is_offline.read());
    let downloaded = move || offline().then_some(true);
    let albums_query = use_memo(move || {
        Some(hooks::AlbumQuery {
            sort: vec![config::SortCriterion::new(
                config::AlbumSortField::Title,
                config::SortDirection::Asc,
            )],
            downloaded: downloaded(),
            ..Default::default()
        })
    });
    let albums_res = use_albums(source, albums_query);
    let new_releases_query = use_memo(move || {
        Some(hooks::AlbumQuery {
            sort: vec![config::SortCriterion::new(
                config::AlbumSortField::Year,
                config::SortDirection::Desc,
            )],
            downloaded: downloaded(),
            ..Default::default()
        })
    });
    let new_releases_res = use_albums_window(source, new_releases_query, SHELF_WINDOW);
    let recently_added_query = use_memo(move || {
        Some(hooks::AlbumQuery {
            sort: vec![config::SortCriterion::new(
                config::AlbumSortField::RecentlyAdded,
                config::SortDirection::Desc,
            )],
            downloaded: downloaded(),
            ..Default::default()
        })
    });
    let recently_added_res = use_albums_window(source, recently_added_query, SHELF_WINDOW);
    let artists_query = use_memo(hooks::ArtistQuery::default);
    let artists_res = use_artists(source, artists_query);
    // Photos by artist key, so the Top Artists row shows the picture the daemon holds for each.
    let artist_covers = use_memo(move || {
        artists_res
            .read()
            .clone()
            .unwrap_or_default()
            .iter()
            .filter_map(|artist| {
                let cover =
                    hooks::artwork::url(artist.artwork.as_ref(), hooks::artwork::Size::Thumb)?;
                Some((artist.key.clone(), cover))
            })
            .collect::<HashMap<String, utils::CoverUrl>>()
    });
    let playlists_res = use_playlists();
    let offline_tracks_res = use_downloaded_tracks(source, move || *offline.read());
    // Recently-played for the active source (each source keeps its own history).
    let played_res = hooks::use_db_queries::use_recently_played(source);
    let recent_tracks = use_memo(move || {
        let played = played_res.read().clone().unwrap_or_default();
        if !offline() {
            return played;
        }
        let downloaded: std::collections::HashSet<String> = offline_tracks_res
            .read()
            .iter()
            .flatten()
            .flatten()
            .map(|track| track.key.clone())
            .collect();
        played
            .into_iter()
            .filter(|track| downloaded.contains(&track.key))
            .collect::<Vec<_>>()
    });
    // Only the albums the recent tracks are filed under, for the hero and Continue Listening.
    let recent_albums_query = use_memo(move || {
        let recent = recent_tracks.read();
        let mut ids: Vec<String> = Vec::new();
        for track in recent.iter() {
            if !track.album_id.trim().is_empty() && !ids.contains(&track.album_id) {
                ids.push(track.album_id.clone());
            }
        }
        Some(hooks::AlbumQuery {
            ids: Some(ids),
            ..Default::default()
        })
    });
    let recent_albums_res = use_albums(source, recent_albums_query);
    let top_genre_res = use_top_genre(source);
    let made_for_you_query = use_memo(move || {
        let genre = top_genre_res.read().clone().flatten()?;
        Some(hooks::AlbumQuery {
            genre: Some(genre),
            downloaded: downloaded(),
            ..Default::default()
        })
    });
    let made_for_you_res = use_albums_window(source, made_for_you_query, SHELF_WINDOW);
    let artist_samples_res = use_artist_sample_tracks(source, 30);
    let read_error = use_memo(move || {
        let albums = albums_res.read().clone().and_then(Result::err);
        let downloaded = offline_tracks_res.read().clone().and_then(Result::err);
        albums.or(downloaded)
    });

    let source_albums_all = use_memo(move || -> Vec<AlbumCard> {
        // Title order comes from the daemon.
        let albums = albums_res
            .read()
            .clone()
            .and_then(Result::ok)
            .unwrap_or_default();
        albums
            .iter()
            .filter(|album| is_known(album))
            .map(album_card)
            .collect()
    });

    let shuffled_albums = use_memo(move || {
        let albums = source_albums_all();
        if albums.is_empty() {
            return Vec::new();
        }
        let mut rng = rng();
        let mut shuffled = albums.clone();
        shuffled.shuffle(&mut rng);
        shuffled
    });

    // Newest year first, from the daemon.
    let new_releases = use_memo(move || -> Vec<AlbumCard> {
        let albums = new_releases_res
            .read()
            .clone()
            .and_then(Result::ok)
            .unwrap_or_default();
        shelf(&albums)
    });

    // Newest first, from the daemon.
    let recently_added = use_memo(move || -> Vec<AlbumCard> {
        let albums = recently_added_res
            .read()
            .clone()
            .and_then(Result::ok)
            .unwrap_or_default();
        shelf(&albums)
    });

    let recent_albums = use_memo(move || {
        recent_albums_res
            .read()
            .clone()
            .and_then(Result::ok)
            .unwrap_or_default()
    });

    let continue_listening = use_memo(move || {
        let recent_tracks = recent_tracks();
        let recent_albums = recent_albums();
        let album_by_id: HashMap<&str, &Album> =
            recent_albums.iter().map(|a| (a.id.as_str(), a)).collect();
        let mut out: Vec<(Track, Option<Album>, Option<String>)> = Vec::new();
        let mut seen_albums = std::collections::HashSet::new();
        for track in recent_tracks.iter() {
            if track.title.trim().is_empty() {
                continue;
            }
            let album = album_by_id.get(track.album_id.as_str()).copied().cloned();
            if let Some(ref album_ref) = album {
                if !is_known(album_ref) {
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
            let cover = track_cover_url(track);
            out.push((track.clone(), album, cover));
            if out.len() >= 10 {
                break;
            }
        }
        out
    });

    let hero_entry = use_memo(move || {
        let recent_tracks = recent_tracks();
        let recent_albums = recent_albums();
        let album_by_id: HashMap<&str, &Album> =
            recent_albums.iter().map(|a| (a.id.as_str(), a)).collect();

        recent_tracks
            .iter()
            .find_map(|track| featured_album(track, &album_by_id))
    });

    let made_for_you = use_memo(move || -> (String, Vec<AlbumCard>) {
        let Some(top_genre) = top_genre_res.read().clone().flatten() else {
            return (String::new(), Vec::new());
        };
        let albums = made_for_you_res
            .read()
            .clone()
            .and_then(Result::ok)
            .unwrap_or_default();
        (top_genre, shelf(&albums))
    });

    let source_artists = use_memo(move || {
        let tracks = if offline() {
            // By artist, from the daemon.
            offline_tracks_res
                .read()
                .clone()
                .and_then(Result::ok)
                .unwrap_or_default()
        } else {
            artist_samples_res.read().clone().unwrap_or_default()
        };
        let mut unique_artists = std::collections::HashSet::new();
        let mut artist_list = Vec::new();
        for track in &tracks {
            // The row's own credit, so the tile is the artist the source named
            // rather than the billed string it happens to show.
            let Some(credit) = track.primary_credit() else {
                continue;
            };
            if is_unknown_artist(&credit.name) {
                continue;
            }
            let Some(key) = &credit.key else {
                continue;
            };
            if unique_artists.insert(key.clone()) {
                // The daemon walks override, then photo, then an album cover
                // for a library source; no picture renders the placeholder.
                let cover_url = artist_covers
                    .read()
                    .get(key)
                    .map(|cover: &utils::CoverUrl| cover.as_ref().to_string());
                artist_list.push((credit.name.clone(), cover_url, key.clone()));
            }
            if artist_list.len() >= 10 {
                break;
            }
        }
        artist_list
    });

    let recent_playlists = use_memo(move || {
        let store = playlists_res.read().clone().unwrap_or_default();
        let offline = offline();
        let downloaded: std::collections::HashSet<String> = offline_tracks_res
            .read()
            .iter()
            .flatten()
            .flatten()
            .map(|t| t.key.clone())
            .collect();
        store
            .playlists
            .iter()
            .filter(|p| {
                if !offline {
                    return true;
                }
                !p.track_keys.is_empty() && p.track_keys.iter().all(|tid| downloaded.contains(tid))
            })
            .rev()
            .take(10)
            .cloned()
            .map(|p| {
                // The daemon walked the playlist's own cover, its server's and
                // the first track's, so the row's reference is the whole answer.
                let cover_url =
                    hooks::artwork::url(p.artwork.as_ref(), hooks::artwork::Size::Thumb)
                        .map(|cover| cover.to_string());
                (p.id, p.name, p.track_keys.len(), cover_url)
            })
            .collect::<Vec<_>>()
    });

    let hero_cover = use_memo(move || {
        let entry = hero_entry.read();
        let (track, album_opt, _) = entry.as_ref()?;
        // The album's own art first, but fall back to the track's — the albums
        // query lags the recently-played one, and not every album has a cover
        // path, which otherwise left the hero on the 384px card thumbnail.
        let cover = album_opt
            .as_ref()
            .and_then(|album| hooks::artwork::for_album(album, hooks::artwork::Size::Full))
            .or_else(|| hooks::artwork::for_track(track, hooks::artwork::Size::Full))?;
        Some(cover.to_string())
    });

    let is_vaxry = settings.read().ui.ui_style == UiStyle::Vaxry;
    let prefs_snapshot = prefs.read();
    let listen_now_style = prefs_snapshot.listen_now_style;
    let sections: Vec<(String, bool)> = prefs_snapshot
        .home_sections
        .iter()
        .map(|s| (s.key.clone(), s.enabled))
        .collect();
    drop(prefs_snapshot);

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
            if let Some(error) = read_error() {
                crate::layout::ReadError { error }
            }
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
                                    div { class: "app-card app-card-row flex items-center justify-between gap-2 mb-2 px-2 py-2 rounded-lg bg-white/5 border border-white/10",
                                        div { class: "flex items-center gap-2 text-white/80 text-xs font-bold",
                                            i { class: "fa-solid fa-grip-vertical text-white/30" }
                                            span { "{section_label(&key)}" }
                                        }
                                        div { class: "flex items-center gap-1",
                                            if key == "listen_now" {
                                                button {
                                                    class: "app-button-tonal px-3 h-7 rounded-md bg-white/5 hover:bg-white/15 text-white/70 hover:text-white text-xs font-semibold transition-colors",
                                                    title: i18n::t("listen_now_layout").to_string(),
                                                    onclick: move |_| {
                                                        let mut conf = prefs.write();
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
                                                class: "app-icon-button w-7 h-7 rounded-md bg-white/5 hover:bg-white/15 text-white/70 hover:text-white transition-colors",
                                                title: i18n::t("move_up").to_string(),
                                                disabled: idx == 0,
                                                onclick: move |_| {
                                                    let mut conf = prefs.write();
                                                    if let Some(i) = conf.home_sections.iter().position(|s| s.key == key_up)
                                                        && i > 0 { conf.home_sections.swap(i, i - 1); }
                                                },
                                                i { class: "fa-solid fa-chevron-up text-xs" }
                                            }
                                            button {
                                                class: "app-icon-button w-7 h-7 rounded-md bg-white/5 hover:bg-white/15 text-white/70 hover:text-white transition-colors",
                                                title: i18n::t("move_down").to_string(),
                                                disabled: idx + 1 >= total,
                                                onclick: move |_| {
                                                    let mut conf = prefs.write();
                                                    if let Some(i) = conf.home_sections.iter().position(|s| s.key == key_down)
                                                        && i + 1 < conf.home_sections.len() { conf.home_sections.swap(i, i + 1); }
                                                },
                                                i { class: "fa-solid fa-chevron-down text-xs" }
                                            }
                                            button {
                                                class: if enabled {
                                                    "app-button-tonal px-3 h-7 rounded-md bg-indigo-500/20 hover:bg-indigo-500/30 text-indigo-300 text-xs font-semibold transition-colors"
                                                } else {
                                                    "app-button-outlined px-3 h-7 rounded-md bg-white/5 hover:bg-white/15 text-white/60 text-xs font-semibold transition-colors"
                                                },
                                                onclick: move |_| {
                                                    let mut conf = prefs.write();
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
                                    settings,
                                    edit,
                                    is_vaxry,
                                    listen_now_style,
                                    shuffled_albums(),
                                    hero_cover(),
                                    continue_listening(),
                                    hero_entry(),
                                    source_artists(),
                                    new_releases(),
                                    made_for_you(),
                                    recently_added(),
                                    recent_playlists(),
                                    on_select_album,
                                    on_play_album,
                                    on_select_playlist,
                                    on_open_artist,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn track(title: &str, album: &str, album_id: &str) -> Track {
        Track {
            title: title.to_string(),
            artist: "Ado".to_string(),
            album: album.to_string(),
            album_id: album_id.to_string(),
            ..Track::default()
        }
    }

    #[test]
    fn the_hero_headline_is_the_album_not_the_track() {
        let album = Album {
            id: "a1".to_string(),
            title: "Kyougen".to_string(),
            artist: "Ado".to_string(),
            ..Album::default()
        };
        let played = track("Usseewa", "Kyougen", "a1");
        let albums = HashMap::from([("a1", &album)]);
        let (_, featured, _) = featured_album(&played, &albums).expect("an album");
        let (title, artist) = featured_album_billing(&played, featured.as_ref());
        assert_eq!(title, "Kyougen");
        assert_eq!(artist, "Ado");
    }

    #[test]
    fn the_hero_falls_back_to_the_track_album_name_before_albums_load() {
        let played = track("Usseewa", "Kyougen", "a1");
        let (title, _) = featured_album_billing(&played, None);
        assert_eq!(title, "Kyougen");
    }

    #[test]
    fn a_track_under_no_album_is_never_featured() {
        let albums = HashMap::new();
        assert!(featured_album(&track("Link", "", ""), &albums).is_none());
        assert!(featured_album(&track("Link", "Unknown Album", "dir1"), &albums).is_none());
    }
}
