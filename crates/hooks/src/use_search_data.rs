use config::{AppConfig, MusicService};
use dioxus::prelude::*;
use reader::models::{Album, Track};
use tracing::Instrument;

type TrackRes = Vec<(Track, Option<utils::CoverUrl>)>;
type AlbumRes = Vec<(Album, Option<utils::CoverUrl>)>;

#[derive(Clone, Copy)]
pub struct SearchData {
    pub genres: Memo<Vec<(String, Option<utils::CoverUrl>)>>,
    pub search_results: Resource<Option<(TrackRes, AlbumRes)>>,
    pub search_query: Signal<String>,
}

pub fn use_search_data(search_query: Signal<String>, config: Signal<AppConfig>) -> SearchData {
    let active_source = use_context::<Signal<::server::source::ActiveSource>>();
    let source = use_memo(move || config.read().active_source.clone());
    let albums_res = crate::use_db_queries::use_albums(source);
    let gens = crate::db_reactivity::use_generations();

    let genres = use_memo(move || {
        let conf = config.read();
        let active_source = conf.active_source.clone();
        let active_service = conf.active_service();
        let server = conf.server.clone();
        let albums = albums_res.read().clone().unwrap_or_default();

        if active_source.is_server() {
            let mut genre_items = std::collections::HashMap::new();
            for album in &albums {
                for g in album.genre.split(['/', ';', ',']) {
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
                                    Some(MusicService::YtMusic)
                                    | Some(MusicService::SoundCloud)
                                    | None => None,
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

        for album in &albums {
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
        let _ = gens.generation(crate::db_reactivity::Table::Tracks);
        let _ = gens.generation(crate::db_reactivity::Table::Albums);
        let query = search_query.read().to_lowercase();
        // The source owns search: local/Jellyfin/Subsonic filter their corpus,
        // YT queries its catalog (see `MediaSource::search`). Covers are resolved
        // here through the cover seam, which dispatches on the source/track.
        let conf = config.read().clone();
        let source = active_source.read().clone();
        let all_albums = albums_res.read().clone().unwrap_or_default();

        async move {
            if query.trim().is_empty() {
                return None;
            }
            let span = tracing::info_span!("query.search", source = conf.active_source.as_str());
            let (tracks, albums) = source.search(&query).instrument(span).await.ok()?;
            let album_cover: std::collections::HashMap<&String, Option<&std::path::Path>> =
                all_albums
                    .iter()
                    .map(|a| (&a.id, a.cover_path.as_deref()))
                    .collect();
            let result_tracks: TrackRes = tracks
                .iter()
                .map(|t| {
                    let ac = album_cover.get(&t.album_id).copied().flatten();
                    (t.clone(), server::cover::track(&conf, t, ac, 80))
                })
                .collect();
            let result_albums: AlbumRes = albums
                .iter()
                .map(|a| {
                    (
                        a.clone(),
                        server::cover::from_path(&conf, a.cover_path.as_deref(), 360),
                    )
                })
                .collect();
            Some((result_tracks, result_albums))
        }
    });

    SearchData {
        genres,
        search_results,
        search_query,
    }
}
