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
use config::Source;
use dioxus::prelude::*;
use tracing::Instrument;

use crate::api::use_api;
use crate::db_reactivity::{Table, use_generations};
use crate::wire::{albums_from_api, tracks_from_api};

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

/// Everything, for the call sites that genuinely want the whole list (playing
/// a filtered view, resolving a playlist). The daemon still pages internally.
pub fn all() -> Page {
    Page {
        offset: 0,
        limit: u32::MAX,
    }
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
                    .map(|page| tracks_from_api(page.items))
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
    source: Memo<Source>,
    album_id: Memo<String>,
) -> Resource<Vec<reader::Track>> {
    let api = use_api();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Tracks);
        let (api, s, id) = (api.clone(), source(), album_id());
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
                .map(|page| tracks_from_api(page.items))
                .unwrap_or_default();
            tracing::Span::current().record("rows", rows.len());
            rows
        }
        .instrument(span)
    })
}

/// Every track credited to an artist.
pub fn use_artist_tracks(
    source: Memo<Source>,
    artist: Memo<String>,
) -> Resource<Vec<reader::Track>> {
    let api = use_api();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Tracks);
        let (api, s, name) = (api.clone(), source(), artist());
        let span = tracing::info_span!(
            "query.artist_tracks",
            source = s.as_str(),
            artist = %name,
            rows = tracing::field::Empty,
        );
        async move {
            let rows = api
                .artist_tracks(name, all())
                .await
                .map(|page| tracks_from_api(page.items))
                .unwrap_or_default();
            tracing::Span::current().record("rows", rows.len());
            rows
        }
        .instrument(span)
    })
}

/// Every track in a genre. An empty genre resolves to empty without asking.
pub fn use_genre_tracks(source: Memo<Source>, genre: Memo<String>) -> Resource<Vec<reader::Track>> {
    let api = use_api();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Tracks);
        let (api, s, name) = (api.clone(), source(), genre());
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
                .map(|page| tracks_from_api(page.items))
                .unwrap_or_default();
            tracing::Span::current().record("rows", rows.len());
            rows
        }
        .instrument(span)
    })
}

/// One track per artist, for the artist grid's tiles.
pub fn use_artist_sample_tracks(source: Memo<Source>, limit: u32) -> Resource<Vec<reader::Track>> {
    let api = use_api();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Tracks);
        let (api, s) = (api.clone(), source());
        let span = tracing::info_span!("query.artist_sample_tracks", source = s.as_str(), limit);
        async move {
            api.artist_sample_tracks(Page { offset: 0, limit })
                .await
                .map(|page| tracks_from_api(page.items))
                .unwrap_or_default()
        }
        .instrument(span)
    })
}

/// The genre with the most tracks, for the home page's heading.
pub fn use_top_genre(source: Memo<Source>) -> Resource<Option<String>> {
    let api = use_api();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Tracks);
        let (api, s) = (api.clone(), source());
        let span = tracing::info_span!("query.top_genre", source = s.as_str());
        async move { api.top_genre().await.unwrap_or_default() }.instrument(span)
    })
}

/// Resolve tracks by key (recents, playlist refs), preserving input order.
pub fn use_tracks_by_keys(
    source: Memo<Source>,
    keys: Memo<Vec<String>>,
) -> Resource<Vec<reader::Track>> {
    let api = use_api();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Tracks);
        let (api, s, k) = (api.clone(), source(), keys());
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
            let rows = api
                .tracks_by_keys(k)
                .await
                .map(tracks_from_api)
                .unwrap_or_default();
            tracing::Span::current().record("rows", rows.len());
            rows
        }
        .instrument(span)
    })
}

/// This source's recently-played tracks, newest first.
pub fn use_recently_played(source: Memo<Source>) -> Resource<Vec<reader::Track>> {
    let api = use_api();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Recents);
        let (api, s) = (api.clone(), source());
        let span = tracing::info_span!("query.recently_played", source = s.as_str());
        async move {
            api.recent_tracks(Page {
                offset: 0,
                limit: 50,
            })
            .await
            .map(|page| tracks_from_api(page.items))
            .unwrap_or_default()
        }
        .instrument(span)
    })
}

/// One album by id.
pub fn use_album(source: Memo<Source>, album_id: Memo<String>) -> Resource<Option<reader::Album>> {
    let api = use_api();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Albums);
        let (api, s, id) = (api.clone(), source(), album_id());
        let span = tracing::info_span!("query.album", source = s.as_str(), album_id = %id);
        async move {
            api.album(id)
                .await
                .unwrap_or_default()
                .map(crate::wire::album_from_api)
        }
        .instrument(span)
    })
}

/// Distinct artists for a source with track counts, A→Z.
pub fn use_artists(source: Memo<Source>) -> Resource<Vec<(String, u32)>> {
    let api = use_api();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Tracks);
        let (api, s) = (api.clone(), source());
        let span = tracing::info_span!(
            "query.artists",
            source = s.as_str(),
            rows = tracing::field::Empty
        );
        async move {
            let rows: Vec<(String, u32)> = api
                .artists(all())
                .await
                .map(|page| {
                    page.artists
                        .into_iter()
                        .map(|artist| (artist.name, artist.track_count))
                        .collect()
                })
                .unwrap_or_default();
            tracing::Span::current().record("rows", rows.len());
            rows
        }
        .instrument(span)
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
    let api = use_api();
    let gens = use_generations();
    let source = use_active_source();
    use_resource(move || {
        let _ = gens.generation(Table::Playlists);
        let _ = gens.generation(Table::Folders);
        let (api, src) = (api.clone(), source());
        let span = tracing::info_span!("query.playlists", source = %src.as_str());
        async move {
            api.playlists()
                .await
                .map(crate::wire::playlist_store_from_api)
                .unwrap_or_default()
        }
        .instrument(span)
    })
}

/// Per-artist images: `(overrides, photos)` — see [`db::ArtistImages`].
pub fn use_artist_images() -> Resource<db::ArtistImages> {
    let db = use_context::<db::ReadDb>();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Tracks);
        let db = db.clone();
        let span = tracing::info_span!("query.artist_images");
        utils::offload(async move { db.artist_images().await.unwrap_or_default() }.instrument(span))
    })
}

/// All albums for a source, re-queried when the albums table changes.
pub fn use_albums(source: Memo<Source>) -> Resource<Vec<reader::Album>> {
    let api = use_api();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Albums);
        let (api, s) = (api.clone(), source());
        let span = tracing::info_span!(
            "query.albums",
            source = s.as_str(),
            rows = tracing::field::Empty
        );
        async move {
            let rows = api
                .albums(all())
                .await
                .map(|page| albums_from_api(page.albums))
                .unwrap_or_default();
            tracing::Span::current().record("rows", rows.len());
            rows
        }
        .instrument(span)
    })
}

/// A per-track display-cover resolver: call `resolve(&track)` for any track and
/// get its cover, with no source/partition decision at the call site. Every track
/// self-describes its cover via the source-layer seam ([`server::cover::track`])
/// — an API row carries an `artwork://` ref the daemon resolves — so this needs
/// no album lookup; a mixed-source list resolves correctly.
pub fn use_cover_resolver(max_width: u32) -> impl Fn(&reader::Track) -> Option<utils::CoverUrl> {
    let config = use_context::<Signal<config::AppConfig>>();
    move |track: &reader::Track| ::server::cover::track(&config.read(), track, max_width)
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
pub fn use_track_is_favorite(track: Memo<Option<reader::Track>>) -> Memo<bool> {
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
            let key = track.id.key();
            if key.trim().is_empty() {
                return false;
            }
            api.favorites()
                .await
                .map(|view| view.refs.iter().any(|entry| entry == key.as_ref()))
                .unwrap_or(false)
        }
    });
    use_memo(move || res.read().unwrap_or(false))
}
