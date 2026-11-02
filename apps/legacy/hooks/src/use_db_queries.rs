//! Library query hooks.
//!
//! Each hook is a [`use_resource`] keyed on `(its inputs, the table's
//! generation)`: when the daemon reports that table dirty (see
//! [`crate::db_reactivity`]), the resource re-runs and the UI updates. Big
//! lists go through [`use_tracks_window`], which only asks for the visible
//! page, so a 20k-track library scrolls without ever holding the whole list.
//!
//! The queries go to the daemon, not to SQLite. The `source` a caller passes
//! is a reactivity key rather than a parameter -- the daemon reads through
//! whichever source is active -- so a source switch still re-runs everything.
//!
//! Every query runs under a `query.*` span, so the click -> rows-on-screen
//! path is visible in a trace: each re-run is one slice with its inputs and
//! result count.

use api::{Page, TrackFilter};
use dioxus::prelude::*;
use tracing::Instrument;

use crate::api::use_api;
use crate::db_reactivity::{Table, use_generations};

/// One resolved window: the rows together with the offset they were queried
/// at. The pairing matters — a `Resource` keeps its previous value while
/// re-running, so rows labeled with the CURRENT page offset would briefly
/// mislabel (and mis-play) the old window during a scroll.
#[derive(Clone, Default, PartialEq)]
pub struct WindowRows {
    pub offset: u32,
    pub rows: Vec<api::TrackInfo>,
}

/// A windowed track listing: the visible `rows` plus the `total` match count
/// (for the virtual-scroll spacer). Both are `Resource`s — `None` while loading.
#[derive(Clone, Copy)]
pub struct TracksWindow {
    pub rows: Resource<WindowRows>,
    pub total: Resource<u32>,
}

/// Everything, for the call sites that genuinely want the whole list (playing
/// a filtered view, resolving a playlist). The daemon still pages internally.
pub fn all() -> Page {
    Page {
        offset: 0,
        limit: u32::MAX,
    }
}

/// Run `query` for the active source and read its answer only while that source is still the active one, so a switch shows nothing until the new source answers.
fn use_source_query<T, F>(
    source: Memo<String>,
    mut query: impl FnMut(String) -> F + 'static,
) -> Memo<Option<T>>
where
    T: Clone + PartialEq + 'static,
    F: std::future::Future<Output = T> + 'static,
{
    let answered = use_resource(move || {
        let asked = source();
        let answer = query(asked.clone());
        async move { (asked, answer.await) }
    });
    use_memo(move || {
        let current = source();
        answered
            .read()
            .as_ref()
            .filter(|(asked, _)| *asked == current)
            .map(|(_, answer)| answer.clone())
    })
}

/// Window into a track listing. `filter` selects source/sort/search; `page` is
/// the visible slice (wire it from `virtual_scroll`'s `start_index`/window).
pub fn use_tracks_window(filter: Memo<TrackFilter>, page: Memo<Page>) -> TracksWindow {
    let api = use_api();
    let gens = use_generations();

    let rows = use_resource({
        let api = api.clone();
        move || {
            let _ = gens.generation(Table::Tracks);
            let (api, f, p) = (api.clone(), filter(), page());
            // A play reorders a most-played list, and plays announce Recents, not Tracks.
            if f.sort == api::TrackSort::PlayCount {
                let _ = gens.generation(Table::Recents);
            }
            let span = tracing::info_span!(
                "query.tracks_page",
                filter = ?f,
                offset = p.offset,
                limit = p.limit,
                rows = tracing::field::Empty,
            );
            async move {
                let rows = api
                    .tracks(f, p)
                    .await
                    .map(|page| page.items)
                    .unwrap_or_default();
                tracing::Span::current().record("rows", rows.len());
                WindowRows {
                    offset: p.offset,
                    rows,
                }
            }
            .instrument(span)
        }
    });

    let total = use_resource({
        let api = api.clone();
        move || {
            let _ = gens.generation(Table::Tracks);
            let (api, f) = (api.clone(), filter());
            let span = tracing::info_span!("query.tracks_count", filter = ?f, total = tracing::field::Empty);
            async move {
                // One row is enough to learn the total; the daemon counts the
                // match set regardless of the window.
                let total = api
                    .tracks(
                        f,
                        Page {
                            offset: 0,
                            limit: 1,
                        },
                    )
                    .await
                    .map(|page| page.total)
                    .unwrap_or(0);
                tracing::Span::current().record("total", total);
                total
            }
            .instrument(span)
        }
    });

    TracksWindow { rows, total }
}

/// One album's tracks, disc/track-ordered. An empty `album_id` (the home-hero
/// "nothing picked yet" sentinel) resolves to empty without asking.
pub fn use_album_tracks(
    source: Memo<String>,
    album_id: Memo<String>,
) -> Memo<Option<Vec<api::TrackInfo>>> {
    let api = use_api();
    let gens = use_generations();
    use_source_query(source, move |s| {
        let _ = gens.generation(Table::Tracks);
        let (api, id) = (api.clone(), album_id());
        let span = tracing::info_span!(
            "query.album_tracks",
            source = s.as_str(),
            album_id = %id,
            rows = tracing::field::Empty,
        );
        async move {
            if id.is_empty() {
                tracing::Span::current().record("rows", 0);
                return Vec::new();
            }
            let rows = api
                .album_tracks(id, all())
                .await
                .map(|page| page.items)
                .unwrap_or_default();
            tracing::Span::current().record("rows", rows.len());
            rows
        }
        .instrument(span)
    })
}

/// Every track credited to an artist. No artist resolves to empty without asking.
pub fn use_artist_tracks(
    source: Memo<String>,
    artist: Memo<Option<String>>,
) -> Memo<Option<Result<Vec<api::TrackInfo>, api::ApiError>>> {
    let api = use_api();
    let gens = use_generations();
    use_source_query(source, move |s| {
        let _ = gens.generation(Table::Tracks);
        let (api, artist) = (api.clone(), artist());
        let span = tracing::info_span!(
            "query.artist_tracks",
            source = s.as_str(),
            artist = ?artist,
            rows = tracing::field::Empty,
        );
        async move {
            let Some(artist) = artist else {
                tracing::Span::current().record("rows", 0);
                return Ok(Vec::new());
            };
            let rows = api.artist_tracks(artist, all()).await?.items;
            tracing::Span::current().record("rows", rows.len());
            Ok(rows)
        }
        .instrument(span)
    })
}

/// One artist's name, photo, track count and billed albums. No artist resolves to `None`.
pub fn use_artist(
    source: Memo<String>,
    artist: Memo<Option<String>>,
) -> Memo<Option<Option<Result<api::ArtistDetail, api::ApiError>>>> {
    let api = use_api();
    let gens = use_generations();
    use_source_query(source, move |s| {
        let _ = gens.generation(Table::Tracks);
        let _ = gens.generation(Table::Albums);
        let _ = gens.generation(Table::Artists);
        let (api, artist) = (api.clone(), artist());
        let span = tracing::info_span!("query.artist", source = s.as_str(), artist = ?artist);
        async move { Some(api.artist(artist?).await) }.instrument(span)
    })
}

/// Every track in a genre. An empty genre resolves to empty without asking.
pub fn use_genre_tracks(
    source: Memo<String>,
    genre: Memo<String>,
) -> Memo<Option<Vec<api::TrackInfo>>> {
    let api = use_api();
    let gens = use_generations();
    use_source_query(source, move |s| {
        let _ = gens.generation(Table::Tracks);
        let (api, name) = (api.clone(), genre());
        let span = tracing::info_span!(
            "query.genre_tracks",
            source = s.as_str(),
            genre = %name,
            rows = tracing::field::Empty,
        );
        async move {
            if name.is_empty() {
                tracing::Span::current().record("rows", 0);
                return Vec::new();
            }
            let rows = api
                .genre_tracks(name, all())
                .await
                .map(|page| page.items)
                .unwrap_or_default();
            tracing::Span::current().record("rows", rows.len());
            rows
        }
        .instrument(span)
    })
}

/// One track per artist, for the artist grid's tiles.
pub fn use_artist_sample_tracks(
    source: Memo<String>,
    limit: u32,
) -> Memo<Option<Vec<api::TrackInfo>>> {
    let api = use_api();
    let gens = use_generations();
    use_source_query(source, move |s| {
        let _ = gens.generation(Table::Tracks);
        let api = api.clone();
        let span = tracing::info_span!("query.artist_sample_tracks", source = s.as_str(), limit);
        async move {
            api.artist_sample_tracks(Page { offset: 0, limit })
                .await
                .map(|page| page.items)
                .unwrap_or_default()
        }
        .instrument(span)
    })
}

/// The genre with the most tracks, for the home page's heading.
pub fn use_top_genre(source: Memo<String>) -> Memo<Option<Option<String>>> {
    let api = use_api();
    let gens = use_generations();
    use_source_query(source, move |s| {
        let _ = gens.generation(Table::Tracks);
        let api = api.clone();
        let span = tracing::info_span!("query.top_genre", source = s.as_str());
        async move { api.top_genre().await.unwrap_or_default() }.instrument(span)
    })
}

/// Resolve tracks by key (recents, playlist refs), preserving input order.
pub fn use_tracks_by_keys(
    source: Memo<String>,
    keys: Memo<Vec<String>>,
) -> Memo<Option<Vec<api::TrackInfo>>> {
    let api = use_api();
    let gens = use_generations();
    use_source_query(source, move |s| {
        let _ = gens.generation(Table::Tracks);
        let (api, k) = (api.clone(), keys());
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
            let rows = api.tracks_by_keys(k).await.unwrap_or_default();
            tracing::Span::current().record("rows", rows.len());
            rows
        }
        .instrument(span)
    })
}

/// This source's tracks with a downloaded or cached copy, by artist, asked for only while `enabled` says so.
pub fn use_downloaded_tracks(
    source: Memo<String>,
    enabled: impl Fn() -> bool + 'static,
) -> Memo<Option<Result<Vec<api::TrackInfo>, api::ApiError>>> {
    let api = use_api();
    let gens = use_generations();
    use_source_query(source, move |s| {
        let _ = gens.generation(Table::Tracks);
        let (api, on) = (api.clone(), enabled());
        let span = tracing::info_span!(
            "query.downloaded_tracks",
            source = s.as_str(),
            rows = tracing::field::Empty,
        );
        async move {
            if !on {
                tracing::Span::current().record("rows", 0);
                return Ok(Vec::new());
            }
            let filter = TrackFilter {
                offline: Some(true),
                sort: api::TrackSort::Artist,
                ..Default::default()
            };
            let rows = api.tracks(filter, all()).await?.items;
            tracing::Span::current().record("rows", rows.len());
            Ok(rows)
        }
        .instrument(span)
    })
}

/// This source's recently-played tracks, newest first.
pub fn use_recently_played(source: Memo<String>) -> Memo<Option<Vec<api::TrackInfo>>> {
    let api = use_api();
    let gens = use_generations();
    use_source_query(source, move |s| {
        let _ = gens.generation(Table::Recents);
        let api = api.clone();
        let span = tracing::info_span!("query.recently_played", source = s.as_str());
        async move {
            api.recent_tracks(Page {
                offset: 0,
                limit: 50,
            })
            .await
            .map(|page| page.items)
            .unwrap_or_default()
        }
        .instrument(span)
    })
}

/// One album by id.
pub fn use_album(
    source: Memo<String>,
    album_id: Memo<String>,
) -> Memo<Option<Option<api::AlbumInfo>>> {
    let api = use_api();
    let gens = use_generations();
    use_source_query(source, move |s| {
        let _ = gens.generation(Table::Albums);
        let (api, id) = (api.clone(), album_id());
        let span = tracing::info_span!("query.album", source = s.as_str(), album_id = %id);
        async move { api.album(id).await.unwrap_or_default() }.instrument(span)
    })
}

/// The artists `query` selects for a source, with track and album counts and photos.
pub fn use_artists(
    source: Memo<String>,
    query: Memo<api::ArtistQuery>,
) -> Memo<Option<Vec<api::ArtistInfo>>> {
    let api = use_api();
    let gens = use_generations();
    use_source_query(source, move |s| {
        let _ = gens.generation(Table::Tracks);
        let _ = gens.generation(Table::Albums);
        let _ = gens.generation(Table::Artists);
        let (api, q) = (api.clone(), query());
        let span = tracing::info_span!(
            "query.artists",
            source = s.as_str(),
            query = ?q,
            rows = tracing::field::Empty
        );
        async move {
            let rows = api
                .artists(q, all())
                .await
                .map(|page| page.artists)
                .unwrap_or_default();
            tracing::Span::current().record("rows", rows.len());
            rows
        }
        .instrument(span)
    })
}

/// The id of the active source, empty until the daemon has listed them.
pub fn use_active_source() -> Memo<String> {
    let active = crate::sources::use_active_source_info();
    use_memo(move || {
        active
            .read()
            .as_ref()
            .map(|source| source.id.clone())
            .unwrap_or_default()
    })
}

/// The active source's play counts by track uid, re-read when tracks or
/// recents change or the source does.
pub fn use_listen_counts(
    source: Memo<String>,
) -> Memo<Option<std::collections::HashMap<String, u64>>> {
    let api = use_api();
    let gens = use_generations();
    use_source_query(source, move |s| {
        let _ = (s, gens.generation(Table::Tracks));
        let _ = gens.generation(Table::Recents);
        let api = api.clone();
        async move {
            api.stats()
                .await
                .map(|stats| stats.listen_counts)
                .unwrap_or_default()
        }
    })
}

/// The playlist catalog for the active source, re-queried on a
/// playlists/folders bump or a source switch.
pub fn use_playlists() -> Memo<Option<api::PlaylistCatalog>> {
    let api = use_api();
    let gens = use_generations();
    let source = use_active_source();
    use_source_query(source, move |s| {
        let _ = gens.generation(Table::Playlists);
        let _ = gens.generation(Table::Folders);
        let (api, src) = (api.clone(), s);
        let span = tracing::info_span!("query.playlists", source = %src.as_str());
        async move { api.playlists().await.unwrap_or_default() }.instrument(span)
    })
}

/// Every album `query` selects, in its order, re-queried when the albums table changes; no query asks for nothing.
pub fn use_albums(
    source: Memo<String>,
    query: Memo<Option<api::AlbumQuery>>,
) -> Memo<Option<Result<Vec<api::AlbumInfo>, api::ApiError>>> {
    use_album_listing(source, query, all())
}

/// The first `limit` albums `query` selects, for a shelf that only shows the top of an order.
pub fn use_albums_window(
    source: Memo<String>,
    query: Memo<Option<api::AlbumQuery>>,
    limit: u32,
) -> Memo<Option<Result<Vec<api::AlbumInfo>, api::ApiError>>> {
    use_album_listing(source, query, Page { offset: 0, limit })
}

fn use_album_listing(
    source: Memo<String>,
    query: Memo<Option<api::AlbumQuery>>,
    page: Page,
) -> Memo<Option<Result<Vec<api::AlbumInfo>, api::ApiError>>> {
    let api = use_api();
    let gens = use_generations();
    use_source_query(source, move |s| {
        let _ = gens.generation(Table::Albums);
        let (api, q) = (api.clone(), query());
        // A track-derived order moves when tracks do, with no album row changing.
        if q.iter().flat_map(|q| &q.sort).any(|sort| {
            matches!(
                sort.field,
                config::AlbumSortField::RecentlyAdded | config::AlbumSortField::TrackCount
            )
        }) {
            let _ = gens.generation(Table::Tracks);
        }
        let span = tracing::info_span!(
            "query.albums",
            source = s.as_str(),
            query = ?q,
            rows = tracing::field::Empty
        );
        async move {
            let Some(q) = q else {
                tracing::Span::current().record("rows", 0);
                return Ok(Vec::new());
            };
            let rows = api.albums(q, page).await?.albums;
            tracing::Span::current().record("rows", rows.len());
            Ok(rows)
        }
        .instrument(span)
    })
}

/// The active source's favorite refs, re-queried on a favorites bump or a
/// source switch.
pub fn use_favorites() -> Resource<Vec<String>> {
    let api = use_api();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Favorites);
        let api = api.clone();
        async move {
            api.favorites()
                .await
                .map(|view| view.refs)
                .unwrap_or_default()
        }
    })
}

/// Whether a single track is favorited — re-run on track change, a favorites
/// bump, or a source swap. Returns a `Memo<bool>`: the in-flight state
/// collapses to `false` (hollow heart until known) here, once, so call sites
/// just read the bool. Use this for single-item checks; a list view should load
/// [`use_favorites`] once and test membership instead.
pub fn use_track_is_favorite(track: Memo<Option<api::TrackInfo>>) -> Memo<bool> {
    let api = use_api();
    let gens = use_generations();
    let res = use_resource(move || {
        let _ = gens.generation(Table::Favorites);
        let api = api.clone();
        let track = track();
        async move {
            let Some(track) = track else {
                return false;
            };
            let key = track.key;
            if key.trim().is_empty() {
                return false;
            }
            api.favorites()
                .await
                .map(|view| view.refs.iter().any(|entry| entry == &key))
                .unwrap_or(false)
        }
    });
    use_memo(move || res.read().unwrap_or(false))
}
