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

fn search_local(
    query: &str,
    tracks: Vec<Track>,
    albums: Vec<Album>,
) -> Option<(TrackRes, AlbumRes)> {
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
            (
                conf.active_source.clone(),
                conf.active_service(),
                conf.server.clone(),
            )
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

#[cfg(test)]
mod tests {
    use super::*;
    use config::{MusicServer, MusicService};
    use std::path::PathBuf;

    fn sample_track(path: &str, album_id: &str, title: &str, artist: &str, album: &str) -> Track {
        Track {
            path: PathBuf::from(path),
            album_id: album_id.to_string(),
            title: title.to_string(),
            artist: artist.to_string(),
            album: album.to_string(),
            duration: 180,
            khz: 44_100,
            bitrate: 320,
            track_number: Some(1),
            disc_number: Some(1),
            musicbrainz_release_id: None,
            playlist_item_id: None,
            artists: vec![artist.to_string()],
        }
    }

    fn sample_album(
        id: &str,
        title: &str,
        artist: &str,
        genre: &str,
        cover: Option<&str>,
    ) -> Album {
        Album {
            id: id.to_string(),
            title: title.to_string(),
            artist: artist.to_string(),
            genre: genre.to_string(),
            year: 2024,
            cover_path: cover.map(PathBuf::from),
            manual_cover: false,
        }
    }

    #[test]
    fn search_local_matches_tracks_by_album_genre() {
        let tracks = vec![sample_track(
            "/music/one.flac",
            "album-1",
            "Safe Room",
            "Alohaii",
            "Patchwork",
        )];
        let albums = vec![sample_album(
            "album-1",
            "Patchwork",
            "Alohaii",
            "Hyperpop",
            None,
        )];

        let (track_results, album_results) =
            search_local("hyperpop", tracks, albums).expect("search should return results");

        assert_eq!(track_results.len(), 1);
        assert_eq!(track_results[0].0.title, "Safe Room");
        assert_eq!(album_results.len(), 1);
        assert_eq!(album_results[0].0.title, "Patchwork");
    }

    #[test]
    fn search_local_dedupes_albums_case_insensitively_by_trimmed_title() {
        let tracks = vec![];
        let albums = vec![
            sample_album("album-1", " Patchwork ", "Alohaii", "Pop", None),
            sample_album("album-2", "patchwork", "Someone Else", "Rock", None),
            sample_album("album-3", "Different", "Alohaii", "Pop", None),
        ];

        let (_track_results, album_results) =
            search_local("patch", tracks, albums).expect("search should return results");

        assert_eq!(album_results.len(), 1);
        assert_eq!(album_results[0].0.id, "album-1");
    }

    #[test]
    fn search_local_applies_track_and_album_limits() {
        let tracks: Vec<Track> = (0..120)
            .map(|idx| {
                sample_track(
                    &format!("/music/{idx}.flac"),
                    &format!("album-{idx}"),
                    &format!("Match Song {idx}"),
                    "Artist",
                    &format!("Album {idx}"),
                )
            })
            .collect();
        let albums: Vec<Album> = (0..40)
            .map(|idx| {
                sample_album(
                    &format!("album-{idx}"),
                    &format!("Match Album {idx}"),
                    "Artist",
                    "Genre",
                    None,
                )
            })
            .collect();

        let (track_results, album_results) =
            search_local("match", tracks, albums).expect("search should return results");

        assert_eq!(track_results.len(), 100);
        assert_eq!(album_results.len(), 30);
    }

    #[test]
    fn search_local_returns_cover_urls_for_tracks_and_albums_with_cover_paths() {
        let tracks = vec![sample_track(
            "/music/one.flac",
            "album-1",
            "Safe Room",
            "Alohaii",
            "Patchwork",
        )];
        let albums = vec![sample_album(
            "album-1",
            "Patchwork",
            "Alohaii",
            "Pop",
            Some("/music/cover.jpg"),
        )];

        let (track_results, album_results) =
            search_local("patch", tracks, albums).expect("search should return results");

        assert_eq!(track_results.len(), 1);
        assert!(track_results[0].1.is_some());
        assert_eq!(album_results.len(), 1);
        assert!(album_results[0].1.is_some());
    }

    #[test]
    fn search_local_matches_artist_and_album_fields() {
        let tracks = vec![
            sample_track(
                "/music/one.flac",
                "album-1",
                "Safe Room",
                "Alohaii",
                "Patchwork",
            ),
            sample_track("/music/two.flac", "album-2", "Other", "Else", "Different"),
        ];
        let albums = vec![
            sample_album("album-1", "Patchwork", "Alohaii", "Pop", None),
            sample_album("album-2", "Different", "Else", "Rock", None),
        ];

        let (track_results, album_results) =
            search_local("alohaii", tracks, albums).expect("search should return results");

        assert_eq!(track_results.len(), 1);
        assert_eq!(track_results[0].0.artist, "Alohaii");
        assert_eq!(album_results.len(), 1);
        assert_eq!(album_results[0].0.artist, "Alohaii");
    }

    #[test]
    fn search_local_returns_empty_vectors_when_nothing_matches() {
        let tracks = vec![sample_track(
            "/music/one.flac",
            "album-1",
            "Safe Room",
            "Alohaii",
            "Patchwork",
        )];
        let albums = vec![sample_album("album-1", "Patchwork", "Alohaii", "Pop", None)];

        let (track_results, album_results) =
            search_local("does-not-match", tracks, albums).expect("search should return results");

        assert!(track_results.is_empty());
        assert!(album_results.is_empty());
    }

    #[test]
    fn search_local_keeps_duplicate_tracks_without_forcing_album_matches() {
        let track = sample_track(
            "/music/one.flac",
            "album-1",
            "Safe Room",
            "Alohaii",
            "Patchwork",
        );
        let tracks = vec![track.clone(), track];
        let albums = vec![sample_album("album-1", "Patchwork", "Alohaii", "Pop", None)];

        let (track_results, album_results) =
            search_local("safe", tracks, albums).expect("search should return results");

        assert_eq!(track_results.len(), 2);
        assert!(album_results.is_empty());
    }

    #[test]
    fn search_server_returns_none_cover_urls_without_server_context() {
        let tracks = vec![sample_track(
            "jellyfin:track123",
            "album-1",
            "Safe Room",
            "Alohaii",
            "Patchwork",
        )];
        let albums = vec![sample_album(
            "album-1",
            "Patchwork",
            "Alohaii",
            "Pop",
            Some("jellyfin:cover123"),
        )];

        let (track_results, album_results) =
            search_server("safe", tracks, albums, Some(MusicService::Jellyfin), None)
                .expect("search should return results");

        assert_eq!(track_results.len(), 1);
        assert!(track_results[0].1.is_none());
        assert!(album_results.is_empty());
    }

    #[test]
    fn search_server_builds_cover_urls_when_server_context_exists() {
        let tracks = vec![sample_track(
            "jellyfin:track123",
            "album-1",
            "Safe Room",
            "Alohaii",
            "Patchwork",
        )];
        let albums = vec![sample_album(
            "album-1",
            "Patchwork",
            "Alohaii",
            "Pop",
            Some("jellyfin:cover123"),
        )];
        let server = MusicServer {
            id: Some("srv-1".to_string()),
            name: "Test Server".to_string(),
            service: MusicService::Jellyfin,
            url: "https://media.example".to_string(),
            user_id: Some("user".to_string()),
            access_token: Some("token".to_string()),
        };

        let (track_results, album_results) = search_server(
            "patch",
            tracks,
            albums,
            Some(MusicService::Jellyfin),
            Some(server),
        )
        .expect("search should return results");

        assert_eq!(track_results.len(), 1);
        assert!(track_results[0].1.is_some());
        assert_eq!(album_results.len(), 1);
        assert!(album_results[0].1.is_some());
    }

    #[test]
    fn search_server_matches_artist_and_dedupes_album_titles() {
        let tracks = vec![
            sample_track("subsonic:1", "album-1", "Song A", "Ado", "Patchwork"),
            sample_track("subsonic:2", "album-2", "Song B", "Else", "Different"),
        ];
        let albums = vec![
            sample_album(
                "album-1",
                " Patchwork ",
                "Ado",
                "Pop",
                Some("subsonic:cover1"),
            ),
            sample_album(
                "album-3",
                "patchwork",
                "Someone Else",
                "Rock",
                Some("subsonic:cover2"),
            ),
        ];
        let server = MusicServer {
            id: Some("srv-2".to_string()),
            name: "Test Server".to_string(),
            service: MusicService::Subsonic,
            url: "https://media.example".to_string(),
            user_id: Some("user".to_string()),
            access_token: Some("token".to_string()),
        };

        let (track_results, album_results) = search_server(
            "ado",
            tracks,
            albums,
            Some(MusicService::Subsonic),
            Some(server),
        )
        .expect("search should return results");

        assert_eq!(track_results.len(), 1);
        assert_eq!(track_results[0].0.artist, "Ado");
        assert_eq!(album_results.len(), 1);
        assert_eq!(album_results[0].0.id, "album-1");
        assert!(album_results[0].1.is_some());
    }

    #[test]
    fn search_server_without_active_service_keeps_matches_but_no_covers() {
        let tracks = vec![sample_track(
            "jellyfin:track123",
            "album-1",
            "Safe Room",
            "Alohaii",
            "Patchwork",
        )];
        let albums = vec![sample_album(
            "album-1",
            "Patchwork",
            "Alohaii",
            "Pop",
            Some("jellyfin:cover123"),
        )];
        let server = MusicServer {
            id: Some("srv-3".to_string()),
            name: "Test Server".to_string(),
            service: MusicService::Jellyfin,
            url: "https://media.example".to_string(),
            user_id: Some("user".to_string()),
            access_token: Some("token".to_string()),
        };

        let (track_results, album_results) =
            search_server("patch", tracks, albums, None, Some(server))
                .expect("search should return results");

        assert_eq!(track_results.len(), 1);
        assert!(track_results[0].1.is_none());
        assert_eq!(album_results.len(), 1);
        assert!(album_results[0].1.is_none());
    }

    #[test]
    fn search_server_returns_empty_vectors_when_nothing_matches() {
        let tracks = vec![sample_track(
            "jellyfin:track123",
            "album-1",
            "Safe Room",
            "Alohaii",
            "Patchwork",
        )];
        let albums = vec![sample_album(
            "album-1",
            "Patchwork",
            "Alohaii",
            "Pop",
            Some("jellyfin:cover123"),
        )];

        let (track_results, album_results) = search_server(
            "does-not-match",
            tracks,
            albums,
            Some(MusicService::Jellyfin),
            None,
        )
        .expect("search should return results");

        assert!(track_results.is_empty());
        assert!(album_results.is_empty());
    }
}
