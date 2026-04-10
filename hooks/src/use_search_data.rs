use config::{AppConfig, MusicService, MusicSource};
use dioxus::prelude::*;
use reader::Library;
use reader::models::{Album, Track};

// why these, its because code was looking complex and clippy said use type for them to make them look
// good.
type TrackRes = Vec<(Track, Option<String>)>;
type AlbumRes = Vec<(Album, Option<String>)>;

#[derive(Clone, Copy)]
pub struct SearchData {
    pub genres: Memo<Vec<(String, Option<String>)>>,
    pub search_results: Memo<Option<(TrackRes, AlbumRes)>>,
    pub search_query: Signal<String>,
}

pub fn use_search_data(
    library: Signal<Library>,
    search_query: Signal<String>,
    config: Signal<AppConfig>,
) -> SearchData {
    let genres = use_memo(move || {
        let conf = config.read();
        let active_source = conf.active_source.clone();
        let active_service = conf.active_service();
        let server = conf.server.clone();
        let lib = library.read();

        if active_source == MusicSource::Server {
            let mut genre_items = std::collections::HashMap::new();
            for album in &lib.jellyfin_albums {
                for g in album.genre.split(|c| c == '/' || c == ';' || c == ',') {
                    let g = g.trim();
                    if !g.is_empty() && !genre_items.contains_key(g) {
                        let cover_url = if let Some(server) = &server {
                            album.cover_path.as_ref().and_then(|cover_path| {
                                let path_str = cover_path.to_string_lossy();
                                match active_service {
                                    Some(MusicService::Jellyfin) => {
                                        utils::jellyfin_image::jellyfin_image_url_from_path(
                                            &path_str,
                                            &server.url,
                                            server.access_token.as_deref(),
                                            320,
                                            80,
                                        )
                                    }
                                    Some(MusicService::Subsonic) | Some(MusicService::Custom) => {
                                        utils::subsonic_image::subsonic_image_url_from_path(
                                            &path_str,
                                            &server.url,
                                            server.access_token.as_deref(),
                                            320,
                                            80,
                                        )
                                    }
                                    None => None,
                                }
                            })
                        } else {
                            None
                        };
                        genre_items.insert(g.to_string(), cover_url);
                    }
                }
            }
            let mut result: Vec<(String, Option<String>)> = genre_items.into_iter().collect();
            result.sort_by(|a, b| a.0.cmp(&b.0));
            return result;
        }

        let mut genre_covers: std::collections::HashMap<String, Vec<std::path::PathBuf>> =
            std::collections::HashMap::new();

        for album in &lib.albums {
            let genre = album.genre.trim();
            if !genre.is_empty() {
                if let Some(cover) = &album.cover_path {
                    genre_covers
                        .entry(genre.to_string())
                        .or_default()
                        .push(cover.clone());
                } else {
                    genre_covers.entry(genre.to_string()).or_default();
                }
            }
        }

        let mut result: Vec<(String, Option<String>)> = genre_covers
            .into_iter()
            .map(|(g, covers)| {
                let cover_url = if !covers.is_empty() {
                    let idx = (g.len() + covers.len()) % covers.len();
                    let c = &covers[idx];
                    utils::format_artwork_url(Some(c))
                } else {
                    None
                };
                (g, cover_url)
            })
            .collect();

        result.sort_by(|a, b| a.0.cmp(&b.0));
        result
    });

    let search_results = use_memo(move || {
        let query = search_query.read().to_lowercase();
        if query.trim().is_empty() {
            return None;
        }

        let lib = library.read();
        let conf = config.read();
        let active_source = conf.active_source.clone();
        let active_service = conf.active_service();
        let server = conf.server.clone();

        let album_map: std::collections::HashMap<&String, &Album> =
            lib.albums.iter().map(|a| (&a.id, a)).collect();

        let mut tracks: Vec<(Track, Option<String>)> = Vec::new();
        let mut albums: Vec<(Album, Option<String>)> = Vec::new();

        match active_source {
            MusicSource::Local => {
                tracks = lib
                    .tracks
                    .iter()
                    .filter(|t| {
                        t.title.to_lowercase().contains(&query)
                            || t.artist.to_lowercase().contains(&query)
                            || t.album.to_lowercase().contains(&query)
                            || album_map
                                .get(&t.album_id)
                                .map(|a| a.genre.to_lowercase().contains(&query))
                                .unwrap_or(false)
                    })
                    .map(|t| {
                        let cover_url = album_map
                            .get(&t.album_id)
                            .and_then(|a| a.cover_path.as_ref())
                            .and_then(|c| utils::format_artwork_url(Some(c)));
                        (t.clone(), cover_url)
                    })
                    .collect();

                let mut seen_titles = std::collections::HashSet::new();
                albums = lib
                    .albums
                    .iter()
                    .filter(|a| {
                        (a.title.to_lowercase().contains(&query)
                            || a.artist.to_lowercase().contains(&query)
                            || a.genre.to_lowercase().contains(&query))
                            && seen_titles.insert(a.title.trim().to_lowercase())
                    })
                    .map(|a| {
                        let cover_url = a
                            .cover_path
                            .as_ref()
                            .and_then(|c| utils::format_artwork_url(Some(c)));
                        (a.clone(), cover_url)
                    })
                    .collect();
            }
            MusicSource::Server => {
                tracks = lib
                    .jellyfin_tracks
                    .iter()
                    .filter(|t| {
                        t.title.to_lowercase().contains(&query)
                            || t.artist.to_lowercase().contains(&query)
                            || t.album.to_lowercase().contains(&query)
                    })
                    .map(|t| {
                        let cover_url = if let Some(server) = &server {
                            let path_str = t.path.to_string_lossy();
                            match active_service {
                                Some(MusicService::Jellyfin) => {
                                    utils::jellyfin_image::jellyfin_image_url_from_path(
                                        &path_str,
                                        &server.url,
                                        server.access_token.as_deref(),
                                        80,
                                        80,
                                    )
                                }
                                Some(MusicService::Subsonic) | Some(MusicService::Custom) => {
                                    utils::subsonic_image::subsonic_image_url_from_path(
                                        &path_str,
                                        &server.url,
                                        server.access_token.as_deref(),
                                        80,
                                        80,
                                    )
                                }
                                None => None,
                            }
                        } else {
                            None
                        };
                        (t.clone(), cover_url)
                    })
                    .collect();

                let mut seen_titles = std::collections::HashSet::new();
                albums = lib
                    .jellyfin_albums
                    .iter()
                    .filter(|a| {
                        (a.title.to_lowercase().contains(&query)
                            || a.artist.to_lowercase().contains(&query)
                            || a.genre.to_lowercase().contains(&query))
                            && seen_titles.insert(a.title.trim().to_lowercase())
                    })
                    .take(50)
                    .map(|a| {
                        let cover_url = if let Some(server) = &server {
                            a.cover_path.as_ref().and_then(|cover_path| {
                                let path_str = cover_path.to_string_lossy();
                                match active_service {
                                    Some(MusicService::Jellyfin) => {
                                        utils::jellyfin_image::jellyfin_image_url_from_path(
                                            &path_str,
                                            &server.url,
                                            server.access_token.as_deref(),
                                            360,
                                            80,
                                        )
                                    }
                                    Some(MusicService::Subsonic) | Some(MusicService::Custom) => {
                                        utils::subsonic_image::subsonic_image_url_from_path(
                                            &path_str,
                                            &server.url,
                                            server.access_token.as_deref(),
                                            360,
                                            80,
                                        )
                                    }
                                    None => None,
                                }
                            })
                        } else {
                            None
                        };
                        (a.clone(), cover_url)
                    })
                    .collect();
            }
        }

        Some((tracks, albums))
    });

    SearchData {
        genres,
        search_results,
        search_query,
    }
}
