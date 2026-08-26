use config::AppConfig;
use dioxus::prelude::*;
use reader::models::{Album, Track};
use std::sync::Arc;
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
    let api = use_context::<Arc<dyn api::KopuzApi>>();
    let source = use_memo(move || config.read().active_source.clone());
    let albums_res = crate::use_db_queries::use_albums(source);
    let gens = crate::db_reactivity::use_generations();

    let genres = use_memo(move || {
        let conf = config.read();
        let albums = albums_res.read().clone().unwrap_or_default();

        // One representative cover per genre, resolved through the source-agnostic
        // cover seam (it dispatches per source — remote URLs for Jellyfin/Subsonic,
        // local file paths otherwise), so there's no local-vs-server branch.
        let mut genre_items: std::collections::HashMap<String, Option<utils::CoverUrl>> =
            std::collections::HashMap::new();
        for album in &albums {
            for g in album.genre.split(['/', ';', ',']) {
                let g = g.trim();
                if g.is_empty() {
                    continue;
                }
                let entry = genre_items.entry(g.to_string()).or_default();
                if entry.is_none() {
                    *entry = server::cover::from_path(&conf, album.cover_path.as_deref(), 320);
                }
            }
        }
        let mut result: Vec<(String, Option<utils::CoverUrl>)> = genre_items.into_iter().collect();
        result.sort_by(|a, b| a.0.cmp(&b.0));
        result
    });

    let search_results = use_resource(move || {
        let _ = gens.generation(crate::db_reactivity::Table::Tracks);
        let _ = gens.generation(crate::db_reactivity::Table::Albums);
        let query = search_query.read().to_lowercase();
        // The daemon dispatches search to the active source. Covers are
        // resolved here through the source-neutral cover seam.
        let conf = config.read().clone();
        let api = api.clone();

        utils::offload(async move {
            if query.trim().is_empty() {
                return None;
            }
            let span = tracing::info_span!("query.search", source = conf.active_source.as_str());
            let result = api
                .search(query)
                .instrument(span)
                .await
                .inspect_err(|e| tracing::warn!(error = %e, "search failed"))
                .ok()?;
            let result_tracks: TrackRes = result
                .tracks
                .into_iter()
                .map(crate::use_db_queries::track_from_api)
                .map(|track| {
                    let artwork = server::cover::track(&conf, &track, 80);
                    (track, artwork)
                })
                .collect();
            let result_albums: AlbumRes = result
                .albums
                .into_iter()
                .map(crate::use_db_queries::album_from_api)
                .map(|album| {
                    let artwork = server::cover::from_path(&conf, album.cover_path.as_deref(), 360);
                    (album, artwork)
                })
                .collect();
            Some((result_tracks, result_albums))
        })
    });

    SearchData {
        genres,
        search_results,
        search_query,
    }
}
