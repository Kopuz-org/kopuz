//! The artist-photo fetch pipeline, feeding the session
//! [`FetchedArtistImages`](server::cover::FetchedArtistImages) map that
//! [`server::cover::artist`] resolves tiles from.
//!
//! Two shapes, chosen by the source's [`ArtistView`](server::source::ArtistView):
//! a Library server (Jellyfin/Subsonic) has a bulk artist-image listing; a
//! Remote-artist catalog (YT) has no bulk endpoint, so each artist's avatar is
//! resolved individually — a few in flight at a time, results written in as
//! they land so the grid fills progressively, hits persisted to the DB
//! (`artist_images` kind `"server"`) so future runs skip the search.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use db::ReadDb;
use dioxus::prelude::*;
use reader::ArtistImageRef;
use server::cover::FetchedArtistImages;
use server::source::{ActiveSource, ArtistView};
use tracing::Instrument;
use utils::artist::{joined_credit_primary, normalize_artist_key};

use crate::use_db_queries::use_active_source;

/// `metadata_cache` kind marking "searched, no photo exists" per normalized
/// artist name — so a miss isn't re-searched every session.
const ARTIST_PHOTO_MISS_KIND: &str = "artist_photo_miss";
/// How long a recorded miss suppresses a re-search. One day: a newly uploaded
/// artist photo shows up on the next day's first visit.
const ARTIST_PHOTO_MISS_TTL_SECS: i64 = 86_400;

/// Drives the photo fetch for the active source. The page passes its own
/// query resources in (they're `Copy`) so the hook doesn't duplicate them.
pub fn use_artist_photo_fetch(
    albums: Resource<Vec<reader::Album>>,
    sample_tracks: Resource<Vec<reader::Track>>,
    artist_images: Resource<db::ArtistImages>,
) {
    let source = use_active_source();
    let active_source = use_context::<Signal<ActiveSource>>();
    let read_db = use_context::<ReadDb>();
    let caps = use_memo(move || active_source.read().capabilities());
    let mut fetched_artist_images = use_context::<Signal<FetchedArtistImages>>();
    // In-flight guard, and WHICH source a fetch already ran for — keyed by
    // source (not a bool) so switching sources refetches instead of silently
    // reusing the previous source's completion.
    let mut is_fetching = use_signal(|| false);
    let mut fetch_done = use_signal(|| None::<config::Source>);

    // Library servers: one bulk artist-image listing fills the session map.
    use_effect(move || {
        // A Remote-artist source (YT) resolves its avatars per-artist below;
        // this bulk server path would yield nothing for it anyway.
        if !caps().sync || caps().artist_view != ArtistView::Library {
            return;
        }
        if *is_fetching.read() || *fetch_done.read() == Some(source()) {
            return;
        }
        is_fetching.set(true);
        let src = active_source.peek().clone();
        let this_source = source.peek().clone();
        spawn(
            async move {
                let images = src.fetch_artist_images().await.unwrap_or_default();
                fetched_artist_images.write().replace_all(images);
                fetch_done.set(Some(this_source));
                is_fetching.set(false);
            }
            .instrument(tracing::info_span!("artist.fetch_images")),
        );
    });

    // Remote-artist catalogs (YT): per-artist avatar resolution.
    use_effect(move || {
        if caps().artist_view != ArtistView::Remote {
            return;
        }
        if *is_fetching.read() || *fetch_done.read() == Some(source()) {
            return;
        }
        // Wait for the DB artist-image cache to load so artists whose photo was
        // persisted on a previous run are skipped (otherwise every page open
        // would re-search them).
        let db_imgs = artist_images.read();
        let Some((_, db_photos)) = db_imgs.clone() else {
            return;
        };
        drop(db_imgs);

        let albums = albums.read().clone().unwrap_or_default();
        let sample = sample_tracks.read().clone().unwrap_or_default();
        if albums.is_empty() && sample.is_empty() {
            // Library not loaded yet — wait for a real artist set.
            return;
        }
        let names = {
            let already = fetched_artist_images.read();
            fetch_queue(&albums, &sample, &db_photos, &already)
        };
        // Mark done up front so the effect doesn't respawn as the workers write
        // partial results back into the map; mark every queued name pending so
        // its tile shows a placeholder instead of the last resort it'd get as
        // "not fetching".
        fetch_done.set(Some(source.peek().clone()));
        if names.is_empty() {
            return;
        }
        is_fetching.set(true);
        fetched_artist_images
            .write()
            .mark_pending(names.iter().cloned());

        // One coordinator task owns the whole fetch: it settles the TTL'd
        // misses first (only after that does it touch any signal), then drains
        // the remainder with a small worker pool.
        let src = active_source.peek().clone();
        let db = read_db.clone();
        spawn(
            async move {
                let to_fetch = settle_fresh_misses(&db, fetched_artist_images, names).await;
                drain_queue(src, fetched_artist_images, to_fetch).await;
            }
            .instrument(tracing::info_span!("artist.fetch_yt_images")),
        );
        is_fetching.set(false);
    });
}

/// The names the per-artist loop should fetch, in the grid's order.
///
/// Pure: collects every album/track-credit artist, drops joined collab credits
/// whose primary artist is independently present (the grid gives them no tile,
/// so a search is wasted work), skips names already resolved this session or
/// persisted in the DB photo cache, and sorts case-insensitively — the grid's
/// order, not the byte order that queues every lowercase name behind the whole
/// uppercase alphabet.
fn fetch_queue(
    albums: &[reader::Album],
    sample: &[reader::Track],
    db_photos: &HashMap<String, ArtistImageRef>,
    already: &FetchedArtistImages,
) -> Vec<String> {
    let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for album in albums {
        if !album.artist.trim().is_empty() {
            names.insert(album.artist.clone());
        }
    }
    for track in sample {
        for artist in &track.artists {
            if !artist.trim().is_empty() {
                names.insert(artist.clone());
            }
        }
    }
    let norms: std::collections::HashSet<String> =
        names.iter().map(|n| normalize_artist_key(n)).collect();
    let mut names: Vec<String> = names
        .into_iter()
        .filter(|n| {
            let norm = normalize_artist_key(n);
            !joined_credit_primary(&norm).is_some_and(|p| norms.contains(p))
                && !already.contains(n)
                && !db_photos.contains_key(&norm)
        })
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names
}

/// Record the names whose "no photo exists" result is still fresh (within the
/// TTL) as resolved misses, and return the remainder to actually fetch.
async fn settle_fresh_misses(
    db: &ReadDb,
    mut fetched: Signal<FetchedArtistImages>,
    names: Vec<String>,
) -> Vec<String> {
    let fresh_misses: std::collections::HashSet<String> = db
        .meta_keys_since(ARTIST_PHOTO_MISS_KIND, ARTIST_PHOTO_MISS_TTL_SECS)
        .await
        .unwrap_or_default()
        .into_iter()
        .collect();
    let (missed, to_fetch): (Vec<String>, Vec<String>) = names
        .into_iter()
        .partition(|n| fresh_misses.contains(&normalize_artist_key(n)));
    if !missed.is_empty() {
        let mut map = fetched.write();
        for name in missed {
            map.insert_miss(name);
        }
    }
    to_fetch
}

/// Drain the fetch queue with a small pool of identical workers, each pulling
/// the next name off a shared iterator.
async fn drain_queue(src: ActiveSource, fetched: Signal<FetchedArtistImages>, names: Vec<String>) {
    let shared = Arc::new(Mutex::new(names.into_iter()));
    let worker = || {
        let src = src.clone();
        let shared = shared.clone();
        async move {
            while let Some(name) = shared.lock().ok().and_then(|mut it| it.next()) {
                resolve_one(&src, fetched, name).await;
            }
        }
    };
    // Six concurrent lookups — enough to fill a grid page quickly without
    // hammering the catalog. `join!` (not tokio::spawn) keeps the workers on
    // the Dioxus runtime, where the signal writes are allowed.
    tokio::join!(worker(), worker(), worker(), worker(), worker(), worker());
}

/// Resolve one artist's photo and record the outcome — always: the grid must be
/// able to tell "resolved, no photo" (→ the last resort) from "still loading"
/// (→ placeholder).
async fn resolve_one(src: &ActiveSource, mut fetched: Signal<FetchedArtistImages>, name: String) {
    match src.fetch_artist_image(&name).await {
        Ok(Some(url)) => {
            // Persist found photos to the DB (kind "server" → the grid's
            // `photos` map) so future opens load them instantly instead of
            // re-searching.
            let _ = src
                .set_artist_image(&normalize_artist_key(&name), "server", Some(&url))
                .await;
            fetched.write().insert_hit(name, url);
        }
        Ok(None) => {
            // A definitive miss persists (TTL'd) so it isn't re-searched every
            // session.
            let _ = src
                .set_meta(&normalize_artist_key(&name), ARTIST_PHOTO_MISS_KIND, "")
                .await;
            fetched.write().insert_miss(name);
        }
        // A transient error is a session-only miss — it must not hide the
        // artist for a whole TTL.
        Err(_) => fetched.write().insert_miss(name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn album(artist: &str) -> reader::Album {
        reader::Album {
            id: format!("al-{artist}"),
            title: "A".into(),
            artist: artist.into(),
            genre: String::new(),
            year: 0,
            cover_path: None,
            manual_cover: false,
        }
    }

    fn track(artists: &[&str]) -> reader::Track {
        reader::Track {
            id: reader::TrackId::Local("/music/x.flac".into()),
            cover: None,
            album_id: "al".into(),
            title: String::new(),
            artist: artists.first().unwrap_or(&"").to_string(),
            album: String::new(),
            duration: 0,
            khz: 0,
            bitrate: 0,
            track_number: None,
            disc_number: None,
            musicbrainz_release_id: None,
            musicbrainz_recording_id: None,
            musicbrainz_track_id: None,
            playlist_item_id: None,
            artists: artists.iter().map(|a| a.to_string()).collect(),
        }
    }

    #[test]
    fn fetch_queue_filters_and_orders() {
        let albums = [album("Zebra"), album("apple")];
        let sample = [
            track(&["Beta", "COOL&CREATE, beatMARIO"]), // joined credit
            track(&["COOL&CREATE"]),                    // its primary, present
            track(&["  "]),                             // blank credit dropped
        ];
        let mut db_photos: HashMap<String, ArtistImageRef> = HashMap::new();
        db_photos.insert("zebra".into(), ArtistImageRef::Remote("u".into()));
        let mut already = FetchedArtistImages::default();
        already.insert_hit("Beta".into(), "u".into());

        let queue = fetch_queue(&albums, &sample, &db_photos, &already);
        // Zebra: persisted; Beta: resolved this session; the joined credit's
        // primary has its own tile → dropped. Case-insensitive order.
        assert_eq!(queue, vec!["apple".to_string(), "COOL&CREATE".to_string()]);
    }
}
