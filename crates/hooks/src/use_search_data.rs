use config::AppConfig;
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
        // The source owns search: local/Jellyfin/Subsonic filter their corpus,
        // YT queries its catalog (see `MediaSource::search`). Covers are resolved
        // here through the cover seam, which dispatches on the source/track.
        let conf = config.read().clone();
        let source = active_source.read().clone();

        utils::offload(async move {
            if query.trim().is_empty() {
                return None;
            }
            let span = tracing::info_span!("query.search", source = conf.active_source.as_str());
            let (tracks, albums) = source
                .search(&query)
                .instrument(span)
                .await
                // The UI shows the genre fallback on error; without a log the
                // failure is invisible (this hid a Spotify 400 for a while).
                .inspect_err(|e| tracing::warn!(error = %e, "search failed"))
                .ok()?;
            let result_tracks: TrackRes = tracks
                .iter()
                .map(|t| (t.clone(), server::cover::track(&conf, t, 80)))
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
        })
    });

    SearchData {
        genres,
        search_results,
        search_query,
    }
}
