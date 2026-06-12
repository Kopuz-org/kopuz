//! DB-backed query hooks (issue #347, step 6).
//!
//! Each hook is a [`use_resource`] keyed on `(its inputs, the table's
//! generation)`: when a writer bumps that table (see [`crate::db_reactivity`]),
//! the resource re-runs and the UI updates. Big lists go through
//! [`use_tracks_window`], which only materializes the visible page (sort +
//! filter + `LIMIT/OFFSET` happen in SQL), so a 20k-track library scrolls
//! without ever holding the whole list in a signal.
//!
//! Every hook's query runs under a `query.*` span, so the click → rows-on-
//! screen path is visible in a trace: each re-run (input change or generation
//! bump) is one slice with its inputs and result count. The one exception is
//! [`use_is_favorite`] — a per-visible-row point lookup that re-runs for every
//! row on every favorites bump and would bury a trace in micro-slices.

use db::{Db, Page, Source, TrackFilter};
use dioxus::prelude::*;
use tracing::Instrument;

use crate::db_reactivity::{Table, use_generations};

/// A windowed track listing: the visible `rows` plus the `total` match count
/// (for the virtual-scroll spacer). Both are `Resource`s — `None` while loading.
#[derive(Clone, Copy)]
pub struct TracksWindow {
    pub rows: Resource<Vec<reader::Track>>,
    pub total: Resource<u32>,
}

/// Window into a track listing. `filter` selects source/sort/search; `page` is
/// the visible slice (wire it from `virtual_scroll`'s `start_index`/window).
pub fn use_tracks_window(filter: Memo<TrackFilter>, page: Memo<Page>) -> TracksWindow {
    let db = use_context::<Db>();
    let gens = use_generations();

    let rows = use_resource({
        let db = db.clone();
        move || {
            let _ = gens.generation(Table::Tracks);
            let (db, f, p) = (db.clone(), filter(), page());
            let span = tracing::info_span!(
                "query.tracks_page",
                filter = ?f,
                offset = p.offset,
                limit = p.limit,
                rows = tracing::field::Empty,
            );
            async move {
                let rows = db.tracks_page(&f, p).await.unwrap_or_default();
                tracing::Span::current().record("rows", rows.len());
                rows
            }
            .instrument(span)
        }
    });

    let total = use_resource({
        let db = db.clone();
        move || {
            let _ = gens.generation(Table::Tracks);
            let (db, f) = (db.clone(), filter());
            let span =
                tracing::info_span!("query.tracks_count", filter = ?f, total = tracing::field::Empty);
            async move {
                let total = db.tracks_count(&f).await.unwrap_or(0);
                tracing::Span::current().record("total", total);
                total
            }
            .instrument(span)
        }
    });

    TracksWindow { rows, total }
}

/// The complete filtered+sorted list — artist/album details and other small
/// lists. Big unbounded lists should use [`use_tracks_window`] instead.
pub fn use_all_tracks(filter: Memo<TrackFilter>) -> Resource<Vec<reader::Track>> {
    let db = use_context::<Db>();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Tracks);
        let (db, f) = (db.clone(), filter());
        let span =
            tracing::info_span!("query.tracks_all", filter = ?f, rows = tracing::field::Empty);
        async move {
            // Empty-id sentinel (home hero before an album is picked):
            // always zero rows, skip the round-trip.
            if f.album_id.as_deref() == Some("") {
                tracing::Span::current().record("rows", 0);
                return Vec::new();
            }
            let rows = db.tracks_all(&f).await.unwrap_or_default();
            tracing::Span::current().record("rows", rows.len());
            rows
        }
        .instrument(span)
    })
}

/// Resolve tracks by key (recents, playlist refs), preserving input order.
pub fn use_tracks_by_keys(
    source: Memo<Source>,
    keys: Memo<Vec<String>>,
) -> Resource<Vec<reader::Track>> {
    let db = use_context::<Db>();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Tracks);
        let (db, s, k) = (db.clone(), source(), keys());
        let span = tracing::info_span!(
            "query.tracks_by_keys",
            source = s.as_str(),
            keys = k.len(),
            rows = tracing::field::Empty,
        );
        async move {
            if k.is_empty() {
                tracing::Span::current().record("rows", 0);
                return Vec::new();
            }
            let rows = db.tracks_by_keys(&s, &k).await.unwrap_or_default();
            tracing::Span::current().record("rows", rows.len());
            rows
        }
        .instrument(span)
    })
}

/// One album by id.
pub fn use_album(source: Memo<Source>, album_id: Memo<String>) -> Resource<Option<reader::Album>> {
    let db = use_context::<Db>();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Albums);
        let (db, s, id) = (db.clone(), source(), album_id());
        let span = tracing::info_span!("query.album", source = s.as_str(), album_id = %id);
        async move { db.album(&s, &id).await.unwrap_or_default() }.instrument(span)
    })
}

/// Distinct artists for a source with track counts, A→Z.
pub fn use_artists(source: Memo<Source>) -> Resource<Vec<(String, u32)>> {
    let db = use_context::<Db>();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Tracks);
        let (db, s) = (db.clone(), source());
        let span = tracing::info_span!(
            "query.artists",
            source = s.as_str(),
            rows = tracing::field::Empty
        );
        async move {
            let rows = db.artists(&s).await.unwrap_or_default();
            tracing::Span::current().record("rows", rows.len());
            rows
        }
        .instrument(span)
    })
}

/// Distinct album genres for a source, A→Z.
pub fn use_genres(source: Memo<Source>) -> Resource<Vec<String>> {
    let db = use_context::<Db>();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Albums);
        let (db, s) = (db.clone(), source());
        let span = tracing::info_span!(
            "query.genres",
            source = s.as_str(),
            rows = tracing::field::Empty
        );
        async move {
            let rows = db.genres(&s).await.unwrap_or_default();
            tracing::Span::current().record("rows", rows.len());
            rows
        }
        .instrument(span)
    })
}

/// The playlist store (local + active server), re-queried on a playlists bump.
pub fn use_playlists() -> Resource<reader::PlaylistStore> {
    let db = use_context::<Db>();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Playlists);
        let _ = gens.generation(Table::Folders);
        let db = db.clone();
        let span = tracing::info_span!("query.playlists");
        async move { db.load_playlists().await.unwrap_or_default() }.instrument(span)
    })
}

/// The artist image maps: (server urls, local paths, custom paths).
#[allow(clippy::type_complexity)]
pub fn use_artist_images() -> Resource<(
    std::collections::HashMap<String, String>,
    std::collections::HashMap<String, std::path::PathBuf>,
    std::collections::HashMap<String, std::path::PathBuf>,
)> {
    let db = use_context::<Db>();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Tracks);
        let db = db.clone();
        let span = tracing::info_span!("query.artist_images");
        async move { db.artist_images().await.unwrap_or_default() }.instrument(span)
    })
}

/// All albums for a source, re-queried when the albums table changes.
pub fn use_albums(source: Memo<Source>) -> Resource<Vec<reader::Album>> {
    let db = use_context::<Db>();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Albums);
        let (db, s) = (db.clone(), source());
        let span = tracing::info_span!(
            "query.albums",
            source = s.as_str(),
            rows = tracing::field::Empty
        );
        async move {
            let rows = db.albums(&s).await.unwrap_or_default();
            tracing::Span::current().record("rows", rows.len());
            rows
        }
        .instrument(span)
    })
}

/// Favorite refs for a server (`"local"` for filesystem), re-queried on bump.
pub fn use_favorites(server_id: Memo<String>) -> Resource<Vec<String>> {
    let db = use_context::<Db>();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Favorites);
        let (db, sid) = (db.clone(), server_id());
        let span = tracing::info_span!(
            "query.favorites",
            server_id = %sid,
            rows = tracing::field::Empty
        );
        async move {
            let rows = db.favorites(&sid).await.unwrap_or_default();
            tracing::Span::current().record("rows", rows.len());
            rows
        }
        .instrument(span)
    })
}

/// Whether one ref is favorited under a server. Re-queried on a favorites bump,
/// so a toggle anywhere updates every row showing that track.
///
/// Deliberately unspanned: it runs once per visible row per favorites bump —
/// spanning it would flood traces with hundreds of sub-millisecond slices.
pub fn use_is_favorite(server_id: String, ref_: String) -> Resource<bool> {
    let db = use_context::<Db>();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Favorites);
        let (db, sid, r) = (db.clone(), server_id.clone(), ref_.clone());
        async move { db.is_favorite(&sid, &r).await.unwrap_or(false) }
    })
}
