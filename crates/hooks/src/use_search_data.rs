//! Search, and the genre tiles beside it.
//!
//! The source owns search -- a local library filters its own rows, a remote
//! catalog answers over the network -- and the daemon owns the source, so this
//! is one call. It used to reach for the in-process source and resolve every
//! cover here, which meant a frontend needed both the source layer and the
//! credentials that sign a cover URL.

use dioxus::prelude::*;
use tracing::Instrument;

type TrackRes = Vec<(reader::Track, Option<utils::CoverUrl>)>;
type AlbumRes = Vec<(reader::Album, Option<utils::CoverUrl>)>;

#[derive(Clone, Copy)]
pub struct SearchData {
    pub genres: Memo<Vec<(String, Option<utils::CoverUrl>)>>,
    pub search_results: Resource<Option<(TrackRes, AlbumRes)>>,
    pub search_query: Signal<String>,
}

pub fn use_search_data(
    search_query: Signal<String>,
    config: Signal<config::AppConfig>,
) -> SearchData {
    let api = crate::api::use_api();
    let source = use_memo(move || config.read().active_source.clone());
    let albums_res = crate::use_db_queries::use_albums(source);
    let gens = crate::db_reactivity::use_generations();

    // One representative cover per genre, taken from an album that has one --
    // the row already says whether it does, so nothing here has to guess.
    let genres = use_memo(move || {
        let albums = albums_res.read().clone().unwrap_or_default();
        let mut by_genre: std::collections::HashMap<String, Option<utils::CoverUrl>> =
            std::collections::HashMap::new();
        for album in &albums {
            for genre in album.genre.split(['/', ';', ',']) {
                let genre = genre.trim();
                if genre.is_empty() {
                    continue;
                }
                let entry = by_genre.entry(genre.to_string()).or_default();
                if entry.is_none() {
                    *entry = utils::format_artwork_url(album.cover_path.as_deref());
                }
            }
        }
        let mut result: Vec<(String, Option<utils::CoverUrl>)> = by_genre.into_iter().collect();
        result.sort_by(|a, b| a.0.cmp(&b.0));
        result
    });

    let search_results = use_resource(move || {
        let _ = gens.generation(crate::db_reactivity::Table::Tracks);
        let _ = gens.generation(crate::db_reactivity::Table::Albums);
        let query = search_query.read().to_lowercase();
        let api = api.clone();
        async move {
            if query.trim().is_empty() {
                return None;
            }
            let span = tracing::info_span!("query.search");
            let results = api.search(query).instrument(span).await.ok()?;
            let tracks: TrackRes = results
                .tracks
                .into_iter()
                .map(|track| {
                    let cover = crate::wire::artwork_url(track.artwork.as_ref());
                    (crate::wire::track_from_api(track), cover)
                })
                .collect();
            let albums: AlbumRes = results
                .albums
                .into_iter()
                .map(|album| {
                    let cover = crate::wire::artwork_url(album.artwork.as_ref());
                    (crate::wire::album_from_api(album), cover)
                })
                .collect();
            Some((tracks, albums))
        }
    });

    SearchData {
        genres,
        search_results,
        search_query,
    }
}
