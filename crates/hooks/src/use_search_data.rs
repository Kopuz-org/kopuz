use config::{AppConfig, MusicService, MusicSource};
use dioxus::prelude::*;
use reader::Library;
use reader::models::{Album, Track};

type TrackRes = Vec<(Track, Option<utils::CoverUrl>)>;
type AlbumRes = Vec<(Album, Option<utils::CoverUrl>)>;

#[derive(Clone, Copy)]
pub struct SearchData {
    pub genres: Memo<Vec<(String, Option<utils::CoverUrl>)>>,
    pub search_results: Resource<Option<(TrackRes, AlbumRes)>>,
    pub search_query: Signal<String>,
}

fn search_local(query: &str, tracks: Vec<Track>, albums: Vec<Album>) -> Option<(TrackRes, AlbumRes)> {
    let album_map: std::collections::HashMap<&String, &Album> =
        albums.iter().map(|a| (&a.id, a)).collect();

    let result_tracks: TrackRes = tracks
        .iter()
        .filter(|t| {
            t.title.to_lowercase().contains(query)
                || t.artist.to_lowercase().contains(query)
                || t.album.to_lowercase().contains(query)
                || album_map
                    .get(&t.album_id)
                    .map(|a| a.genre.to_lowercase().contains(query))
                    .unwrap_or(false)
        })
        .take(100)
        .map(|t| {
            let cover_url = album_map
                .get(&t.album_id)
                .and_then(|a| a.cover_path.as_ref())
                .and_then(|c| utils::format_artwork_url(Some(c)));
            (t.clone(), cover_url)
        })
        .collect();

    let mut seen = std::collections::HashSet::new();
    let result_albums: AlbumRes = albums
        .iter()
        .filter(|a| {
            (a.title.to_lowercase().contains(query)
                || a.artist.to_lowercase().contains(query)
                || a.genre.to_lowercase().contains(query))
                && seen.insert(a.title.trim().to_lowercase())
        })
        .take(30)
        .map(|a| {
            let cover_url = a
                .cover_path
                .as_ref()
                .and_then(|c| utils::format_artwork_url(Some(c)));
            (a.clone(), cover_url)
        })
        .collect();

    Some((result_tracks, result_albums))
}

fn search_server(
    query: &str,
    tracks: Vec<Track>,
    albums: Vec<Album>,
    active_service: Option<MusicService>,
    server: Option<config::MusicServer>,
) -> Option<(TrackRes, AlbumRes)> {
    let result_tracks: TrackRes = tracks
        .iter()
        .filter(|t| {
            t.title.to_lowercase().contains(query)
                || t.artist.to_lowercase().contains(query)
                || t.album.to_lowercase().contains(query)
        })
        .take(100)
        .map(|t| {
            let cover_url = server.as_ref().and_then(|srv| {
                let path_str = t.path.to_string_lossy();
                let url = match active_service {
                    Some(MusicService::Jellyfin) => {
                        utils::jellyfin_image::jellyfin_image_url_from_path(
                            &path_str,
                            &srv.url,
                            srv.access_token.as_deref(),
                            80,
                            80,
                        )
                    }
                    Some(MusicService::Subsonic) | Some(MusicService::Custom) => {
                        utils::subsonic_image::subsonic_image_url_from_path(
                            &path_str,
                            &srv.url,
                            srv.access_token.as_deref(),
                            80,
                            80,
                        )
                    }
                    None => None,
                };
                utils::map_cover_url(url)
            });
            (t.clone(), cover_url)
        })
        .collect();

    let mut seen = std::collections::HashSet::new();
    let result_albums: AlbumRes = albums
        .iter()
        .filter(|a| {
            (a.title.to_lowercase().contains(query)
                || a.artist.to_lowercase().contains(query)
                || a.genre.to_lowercase().contains(query))
                && seen.insert(a.title.trim().to_lowercase())
        })
        .take(30)
        .map(|a| {
            let cover_url = server.as_ref().and_then(|srv| {
                a.cover_path.as_ref().and_then(|cover_path| {
                    let path_str = cover_path.to_string_lossy();
                    let url = match active_service {
                        Some(MusicService::Jellyfin) => {
                            utils::jellyfin_image::jellyfin_image_url_from_path(
                                &path_str,
                                &srv.url,
                                srv.access_token.as_deref(),
                                360,
                                80,
                            )
                        }
                        Some(MusicService::Subsonic) | Some(MusicService::Custom) => {
                            utils::subsonic_image::subsonic_image_url_from_path(
                                &path_str,
                                &srv.url,
                                srv.access_token.as_deref(),
                                360,
                                80,
                            )
                        }
                        None => None,
                    };
                    utils::map_cover_url(url)
                })
            });
            (a.clone(), cover_url)
        })
        .collect();

    Some((result_tracks, result_albums))
}

#[cfg(not(target_arch = "wasm32"))]
async fn run_search(
    query: String,
    tracks: Vec<Track>,
    albums: Vec<Album>,
    active_source: MusicSource,
    active_service: Option<MusicService>,
    server: Option<config::MusicServer>,
) -> Option<(TrackRes, AlbumRes)> {
    tokio::task::spawn_blocking(move || match active_source {
        MusicSource::Local => search_local(&query, tracks, albums),
        MusicSource::Server => search_server(&query, tracks, albums, active_service, server),
    })
    .await
    .ok()
    .flatten()
}

#[cfg(target_arch = "wasm32")]
async fn run_search(
    query: String,
    tracks: Vec<Track>,
    albums: Vec<Album>,
    active_source: MusicSource,
    active_service: Option<MusicService>,
    server: Option<config::MusicServer>,
) -> Option<(TrackRes, AlbumRes)> {
    match active_source {
        MusicSource::Local => search_local(&query, tracks, albums),
        MusicSource::Server => search_server(&query, tracks, albums, active_service, server),
    }
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
                        genre_items.insert(g.to_string(), utils::map_cover_url(cover_url));
                    }
                }
            }
            let mut result: Vec<(String, Option<utils::CoverUrl>)> =
                genre_items.into_iter().collect();
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

        let mut result: Vec<(String, Option<utils::CoverUrl>)> = genre_covers
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

    let search_results = use_resource(move || {
        let query = search_query.read().to_lowercase();
        let (active_source, active_service, server) = {
            let conf = config.read();
            (conf.active_source.clone(), conf.active_service(), conf.server.clone())
        };
        let (tracks, albums) = {
            let lib = library.read();
            match &active_source {
                MusicSource::Local => (lib.tracks.clone(), lib.albums.clone()),
                MusicSource::Server => (lib.jellyfin_tracks.clone(), lib.jellyfin_albums.clone()),
            }
        };

        async move {
            if query.trim().is_empty() {
                return None;
            }
            run_search(query, tracks, albums, active_source, active_service, server).await
        }
    });

    SearchData {
        genres,
        search_results,
        search_query,
    }
}
