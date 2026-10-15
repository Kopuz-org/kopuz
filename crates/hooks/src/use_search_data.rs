//! Search results from the daemon and genre tiles derived from library albums.

use dioxus::prelude::*;
use tracing::Instrument;

#[derive(Clone, Copy)]
pub struct SearchData {
    pub genres: Memo<Vec<(String, Option<utils::CoverUrl>)>>,
    pub search_results: crate::query::Query<Option<(Vec<api::TrackInfo>, Vec<api::AlbumInfo>)>>,
    pub search_query: Signal<String>,
}

pub fn use_search_data(search_query: Signal<String>) -> SearchData {
    let api = crate::api::use_api();
    let source = crate::use_db_queries::use_active_source();
    let albums_res = crate::use_db_queries::use_albums(source);
    let gens = crate::db_reactivity::use_generations();

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
                    *entry =
                        crate::artwork::url(album.artwork.as_ref(), crate::artwork::Size::Thumb);
                }
            }
        }
        let mut result: Vec<(String, Option<utils::CoverUrl>)> = by_genre.into_iter().collect();
        result.sort_by(|a, b| a.0.cmp(&b.0));
        result
    });

    let search_results = crate::query::use_query(move || {
        let _ = gens.generation(crate::db_reactivity::Table::Tracks);
        let _ = gens.generation(crate::db_reactivity::Table::Albums);
        let query = search_query.read().to_lowercase();
        let api = api.clone();
        async move {
            if query.trim().is_empty() {
                return Ok(None);
            }
            let span = tracing::info_span!("query.search");
            let results = api.search(query).instrument(span).await?;
            Ok(Some((results.tracks, results.albums)))
        }
    });

    SearchData {
        genres,
        search_results,
        search_query,
    }
}
