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
//! bump) is one slice with its inputs and result count.

use db::{Page, ReadDb, Source, TrackFilter};
use dioxus::prelude::*;
use server::source::TrackFavorite;
use tracing::Instrument;
use utils::offload;

use crate::db_reactivity::{Table, use_generations};

/// One resolved window: the rows together with the offset they were queried
/// at. The pairing matters — a `Resource` keeps its previous value while
/// re-running, so rows labeled with the CURRENT page offset would briefly
/// mislabel (and mis-play) the old window during a scroll.
#[derive(Clone, Default, PartialEq)]
pub struct WindowRows {
    pub offset: u32,
    pub rows: Vec<reader::Track>,
}

/// A windowed track listing: the visible `rows` plus the `total` match count
/// (for the virtual-scroll spacer). Both are `Resource`s — `None` while loading.
#[derive(Clone, Copy)]
pub struct TracksWindow {
    pub rows: Resource<WindowRows>,
    pub total: Resource<u32>,
}

/// Window into a track listing. `filter` selects source/sort/search; `page` is
/// the visible slice (wire it from `virtual_scroll`'s `start_index`/window).
pub fn use_tracks_window(filter: Memo<TrackFilter>, page: Memo<Page>) -> TracksWindow {
    let db = use_context::<ReadDb>();
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
            offload(
                async move {
                    let rows = db.tracks_page(&f, p).await.unwrap_or_default();
                    tracing::Span::current().record("rows", rows.len());
                    WindowRows {
                        offset: p.offset,
                        rows,
                    }
                }
                .instrument(span),
            )
        }
    });

    let total = use_resource({
        let db = db.clone();
        move || {
            let _ = gens.generation(Table::Tracks);
            let (db, f) = (db.clone(), filter());
            let span = tracing::info_span!("query.tracks_count", filter = ?f, total = tracing::field::Empty);
            offload(
                async move {
                    let total = db.tracks_count(&f).await.unwrap_or(0);
                    tracing::Span::current().record("total", total);
                    total
                }
                .instrument(span),
            )
        }
    });

    TracksWindow { rows, total }
}

/// One album's tracks, disc/track-ordered. An empty `album_id` (the home-hero
/// "nothing picked yet" sentinel) resolves to empty without touching the DB.
pub fn use_album_tracks(
    source: Memo<Source>,
    album_id: Memo<String>,
) -> Resource<Vec<reader::Track>> {
    let db = use_context::<ReadDb>();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Tracks);
        let (db, s, id) = (db.clone(), source(), album_id());
        let span = tracing::info_span!(
            "query.album_tracks",
            source = s.as_str(),
            album_id = %id,
            rows = tracing::field::Empty,
        );
        offload(
            async move {
                if id.is_empty() {
                    tracing::Span::current().record("rows", 0);
                    return Vec::new();
                }
                let rows = db.album_tracks(&s, &id).await.unwrap_or_default();
                tracing::Span::current().record("rows", rows.len());
                rows
            }
            .instrument(span),
        )
    })
}

/// One artist's tracks, album/disc/track-ordered.
pub fn use_artist_tracks(
    source: Memo<Source>,
    artist: Memo<String>,
) -> Resource<Vec<reader::Track>> {
    let db = use_context::<ReadDb>();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Tracks);
        let (db, s, a) = (db.clone(), source(), artist());
        let span = tracing::info_span!(
            "query.artist_tracks",
            source = s.as_str(),
            artist = %a,
            rows = tracing::field::Empty,
        );
        offload(
            async move {
                let rows = db.artist_tracks(&s, &a).await.unwrap_or_default();
                tracing::Span::current().record("rows", rows.len());
                rows
            }
            .instrument(span),
        )
    })
}

/// Tracks whose album has this genre. An empty genre resolves to empty
/// without touching the DB.
pub fn use_genre_tracks(source: Memo<Source>, genre: Memo<String>) -> Resource<Vec<reader::Track>> {
    let db = use_context::<ReadDb>();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Tracks);
        let (db, s, g) = (db.clone(), source(), genre());
        let span = tracing::info_span!(
            "query.genre_tracks",
            source = s.as_str(),
            genre = %g,
            rows = tracing::field::Empty,
        );
        offload(
            async move {
                if g.is_empty() {
                    tracing::Span::current().record("rows", 0);
                    return Vec::new();
                }
                let rows = db.genre_tracks(&s, &g).await.unwrap_or_default();
                tracing::Span::current().record("rows", rows.len());
                rows
            }
            .instrument(span),
        )
    })
}

/// One representative track per artist, A→Z — artist tiles with covers.
pub fn use_artist_sample_tracks(source: Memo<Source>, limit: u32) -> Resource<Vec<reader::Track>> {
    let db = use_context::<ReadDb>();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Tracks);
        let (db, s) = (db.clone(), source());
        let span = tracing::info_span!(
            "query.artist_samples",
            source = s.as_str(),
            limit,
            rows = tracing::field::Empty,
        );
        offload(
            async move {
                let rows = db.artist_sample_tracks(&s, limit).await.unwrap_or_default();
                tracing::Span::current().record("rows", rows.len());
                rows
            }
            .instrument(span),
        )
    })
}

/// The genre with the highest summed play count for a source.
pub fn use_top_genre(source: Memo<Source>) -> Resource<Option<String>> {
    let db = use_context::<ReadDb>();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Tracks);
        let (db, s) = (db.clone(), source());
        let span = tracing::info_span!("query.top_genre", source = s.as_str());
        offload(async move { db.top_genre(&s).await.unwrap_or_default() }.instrument(span))
    })
}

/// Resolve tracks by key (recents, playlist refs), preserving input order.
pub fn use_tracks_by_keys(
    source: Memo<Source>,
    keys: Memo<Vec<String>>,
) -> Resource<Vec<reader::Track>> {
    let db = use_context::<ReadDb>();
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
        offload(
            async move {
                if k.is_empty() {
                    tracing::Span::current().record("rows", 0);
                    return Vec::new();
                }
                let rows = db.tracks_by_keys(&s, &k).await.unwrap_or_default();
                tracing::Span::current().record("rows", rows.len());
                rows
            }
            .instrument(span),
        )
    })
}

/// This source's recently-played tracks, newest first — reads the recent keys
/// then resolves them, re-running on a source switch or a new play (`Recents`).
pub fn use_recently_played(source: Memo<Source>) -> Resource<Vec<reader::Track>> {
    let db = use_context::<ReadDb>();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Recents);
        let (db, s) = (db.clone(), source());
        let span = tracing::info_span!("query.recently_played", source = s.as_str());
        offload(
            async move {
                let keys = db.recently_played(&s, 50).await.unwrap_or_default();
                if keys.is_empty() {
                    return Vec::new();
                }
                db.tracks_by_keys(&s, &keys).await.unwrap_or_default()
            }
            .instrument(span),
        )
    })
}

/// One album by id.
pub fn use_album(source: Memo<Source>, album_id: Memo<String>) -> Resource<Option<reader::Album>> {
    let db = use_context::<ReadDb>();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Albums);
        let (db, s, id) = (db.clone(), source(), album_id());
        let span = tracing::info_span!("query.album", source = s.as_str(), album_id = %id);
        offload(async move { db.album(&s, &id).await.unwrap_or_default() }.instrument(span))
    })
}

/// Distinct artists for a source with track counts, A→Z.
pub fn use_artists(source: Memo<Source>) -> Resource<Vec<(String, u32)>> {
    let db = use_context::<ReadDb>();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Tracks);
        let (db, s) = (db.clone(), source());
        let span = tracing::info_span!(
            "query.artists",
            source = s.as_str(),
            rows = tracing::field::Empty
        );
        offload(
            async move {
                let rows = db.artists(&s).await.unwrap_or_default();
                tracing::Span::current().record("rows", rows.len());
                rows
            }
            .instrument(span),
        )
    })
}

/// The in-memory active source, straight from the config signal in context —
/// the persisted copy lags a server switch by the debounced save.
pub fn use_active_source() -> Memo<config::Source> {
    let config = use_context::<Signal<config::AppConfig>>();
    use_memo(move || config.read().active_source.clone())
}

/// The playlist store for the active source, re-queried on a playlists/folders
/// bump or a source switch. Resolves the in-memory active source itself.
pub fn use_playlists() -> Resource<reader::PlaylistStore> {
    let db = use_context::<ReadDb>();
    let gens = use_generations();
    let source = use_active_source();
    use_resource(move || {
        let _ = gens.generation(Table::Playlists);
        let _ = gens.generation(Table::Folders);
        let (db, src) = (db.clone(), source());
        let span = tracing::info_span!("query.playlists", source = %src.as_str());
        offload(async move { db.load_playlists(&src).await.unwrap_or_default() }.instrument(span))
    })
}

/// Per-artist images: `(overrides, photos)` — see [`db::ArtistImages`].
pub fn use_artist_images() -> Resource<db::ArtistImages> {
    let db = use_context::<ReadDb>();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Tracks);
        let db = db.clone();
        let span = tracing::info_span!("query.artist_images");
        offload(async move { db.artist_images().await.unwrap_or_default() }.instrument(span))
    })
}

/// All albums for a source, re-queried when the albums table changes.
pub fn use_albums(source: Memo<Source>) -> Resource<Vec<reader::Album>> {
    let db = use_context::<ReadDb>();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Albums);
        let (db, s) = (db.clone(), source());
        let span = tracing::info_span!(
            "query.albums",
            source = s.as_str(),
            rows = tracing::field::Empty
        );
        offload(
            async move {
                let rows = db.albums(&s).await.unwrap_or_default();
                tracing::Span::current().record("rows", rows.len());
                rows
            }
            .instrument(span),
        )
    })
}

/// A per-track display-cover resolver: call `resolve(&track)` for any track and
/// get its cover, with no source/partition decision at the call site. Every track
/// self-describes its cover via the source-layer seam ([`server::cover::track`])
/// — a local row's `cover_path` is projected from its album by the DB read layer
/// — so this needs no album lookup; a mixed-source list resolves correctly.
pub fn use_cover_resolver(max_width: u32) -> impl Fn(&reader::Track) -> Option<utils::CoverUrl> {
    let config = use_context::<Signal<config::AppConfig>>();
    move |track: &reader::Track| ::server::cover::track(&config.read(), track, max_width)
}

/// The active source's favorite refs, re-queried on a favorites bump or a source
/// switch. The favorites partition is the source's own identity, so it's owned by
/// [`MediaSource::favorites`] — callers never spell out a `server_id`.
pub fn use_favorites() -> Resource<Vec<String>> {
    let active_source = use_context::<Signal<::server::source::ActiveSource>>();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Favorites);
        let source = active_source.read().clone();
        offload(async move { source.favorites().await.unwrap_or_default() })
    })
}

/// Whether a single track is favorited — re-run on track change, a favorites
/// bump, or a source swap. Resolved against the active source (the same one the
/// favorite toggle writes through, so display and toggle can't disagree).
/// Returns a `Memo<bool>`: the in-flight state collapses to `false` (hollow
/// heart until known) here, once, so call sites just read the bool. Use this for
/// single-item checks; a list view should load [`use_favorites`] once and test
/// membership instead.
pub fn use_track_is_favorite(track: Memo<Option<reader::Track>>) -> Memo<bool> {
    let active_source = use_context::<Signal<::server::source::ActiveSource>>();
    let gens = use_generations();
    let res = use_resource(move || {
        let _ = gens.generation(Table::Favorites);
        let source = active_source.read().clone();
        let track = track();
        offload(async move {
            match track {
                Some(t) => t.is_favorite(&source).await,
                None => false,
            }
        })
    });
    use_memo(move || res.read().unwrap_or(false))
}
