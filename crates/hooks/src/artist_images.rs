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

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use api::KopuzApi;
use dioxus::prelude::*;
use reader::ArtistImageRef;
use server::cover::FetchedArtistImages;
use server::source::ArtistView;
use tracing::Instrument;
use utils::artist::{joined_credit_primary, normalize_artist_key};

use crate::use_db_queries::use_active_source;

/// Drives the photo fetch for the active source. The page passes its own
/// query resources in (they're `Copy`) so the hook doesn't duplicate them.
pub fn use_artist_photo_fetch(
    albums: Resource<Vec<reader::Album>>,
    sample_tracks: Resource<Vec<reader::Track>>,
    artist_images: Resource<db::ArtistImages>,
) {
    let source = use_active_source();
    let active_source = use_context::<Signal<server::source::ActiveSource>>();
    let api = use_context::<Arc<dyn KopuzApi>>();
    let caps = use_memo(move || active_source.read().capabilities());
    let mut fetched_artist_images = use_context::<Signal<FetchedArtistImages>>();
    // In-flight guard, and WHICH source a fetch already ran for — keyed by
    // source (not a bool) so switching sources refetches instead of silently
    // reusing the previous source's completion.
    let mut is_fetching = use_signal(|| false);
    let mut fetch_done = use_signal(|| None::<config::Source>);

    use_effect(move || {
        if *is_fetching.read() || *fetch_done.read() == Some(source()) {
            return;
        }
        if caps().artist_view == ArtistView::Library && !caps().sync {
            return;
        }
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
        fetch_done.set(Some(source.peek().clone()));
        if names.is_empty() {
            return;
        }
        is_fetching.set(true);
        fetched_artist_images
            .write()
            .mark_pending(names.iter().cloned());

        let api = api.clone();
        spawn(
            async move {
                let available: HashSet<String> = api
                    .refresh_artist_artwork(names.clone())
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .collect();
                let mut fetched = fetched_artist_images.write();
                for name in names {
                    if available.contains(&name) {
                        fetched.insert_hit(
                            name.clone(),
                            crate::use_db_queries::artist_artwork_url(&name),
                        );
                    } else {
                        fetched.insert_miss(name);
                    }
                }
                drop(fetched);
                is_fetching.set(false);
            }
            .instrument(tracing::info_span!("artist.fetch_images")),
        );
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
