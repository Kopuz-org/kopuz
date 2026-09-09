//! Which artists to look for photos of.
//!
//! The search itself is the daemon's -- it holds the credentials, the miss
//! cache and the results. What stays here is the one thing the daemon cannot
//! know: which artists the grid actually shows, which is not simply "every
//! artist in the library". A joined collab credit whose primary artist is
//! independently present gets no tile, so searching for it is wasted work.
//!
//! Results are not returned. The daemon stores what it finds and announces it,
//! so the tiles resolve through the ordinary artwork path on the next read --
//! including in any other frontend that happens to be open.

use dioxus::prelude::*;
use utils::artist::{joined_credit_primary, normalize_artist_key};

/// Ask the daemon to fill in missing artist photos for what this grid renders.
pub fn use_artist_photo_fetch(
    albums: Resource<Vec<api::AlbumInfo>>,
    sample_tracks: Resource<Vec<api::TrackInfo>>,
) {
    use_effect(move || {
        let albums = albums.read().clone().unwrap_or_default();
        let sample = sample_tracks.read().clone().unwrap_or_default();
        // Nothing loaded yet: waiting avoids asking about an empty grid.
        if albums.is_empty() && sample.is_empty() {
            return;
        }
        let names = fetch_queue(&albums, &sample);
        if names.is_empty() {
            return;
        }
        let api = crate::api::consume_api();
        spawn(async move {
            if let Err(error) = api.refresh_artist_artwork(names).await {
                tracing::debug!(%error, "artist artwork refresh failed");
            }
        });
    });
}

/// The artists the grid gives a tile to, in its order.
///
/// Every album and track-credit artist, minus joined collab credits whose
/// primary artist is independently present, sorted case-insensitively.
fn fetch_queue(albums: &[api::AlbumInfo], sample: &[api::TrackInfo]) -> Vec<String> {
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
    let norms: std::collections::HashSet<String> = names
        .iter()
        .map(|name| normalize_artist_key(name))
        .collect();
    let mut names: Vec<String> = names
        .into_iter()
        .filter(|name| {
            let norm = normalize_artist_key(name);
            !joined_credit_primary(&norm).is_some_and(|primary| norms.contains(primary))
        })
        .collect();
    names.sort_by_key(|name| name.to_lowercase());
    names
}
#[cfg(test)]
mod tests {
    use super::*;

    fn album(artist: &str) -> api::AlbumInfo {
        api::AlbumInfo {
            id: format!("al-{artist}"),
            title: "A".into(),
            artist: artist.into(),
            ..Default::default()
        }
    }

    fn track(artists: &[&str]) -> api::TrackInfo {
        api::TrackInfo {
            key: "/music/x.flac".into(),
            uid: "/music/x.flac".into(),
            album_id: "al".into(),
            artist: artists.first().unwrap_or(&"").to_string(),
            artists: artists.iter().map(|a| a.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn fetch_queue_drops_credits_with_no_tile_and_orders_case_insensitively() {
        let albums = [album("Zebra"), album("apple")];
        let sample = [
            track(&["Beta", "COOL&CREATE, beatMARIO"]), // joined credit
            track(&["COOL&CREATE"]),                    // its primary, present
            track(&["  "]),                             // blank credit dropped
        ];

        // The joined credit gets no tile because its primary has one of its
        // own, so searching for it would be wasted. What is already known is
        // the daemon's business, not this list's.
        assert_eq!(
            fetch_queue(&albums, &sample),
            vec![
                "apple".to_string(),
                "Beta".to_string(),
                "COOL&CREATE".to_string(),
                "Zebra".to_string(),
            ]
        );
    }
}
