//! Which artists the grid shows, and asking for their photos.
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
pub fn use_artist_photo_fetch(artists: Resource<Vec<api::ArtistInfo>>) {
    // What the last request asked for. The daemon announces what it finds by
    // dirtying tracks, which is what `artists` is keyed on -- so without
    // remembering the ask, every success re-runs this effect and asks again,
    // forever. A changed grid still gets a fresh request.
    let mut asked_for = use_signal(Vec::<api::ArtistCredit>::new);
    use_effect(move || {
        let artists = artists.read().clone().unwrap_or_default();
        let wanted: Vec<api::ArtistCredit> = grid_artists(&artists)
            .iter()
            .map(|artist| artist.credit())
            .collect();
        if wanted.is_empty() || *asked_for.peek() == wanted {
            return;
        }
        asked_for.set(wanted.clone());
        let api = crate::api::consume_api();
        spawn(async move {
            if let Err(error) = api.refresh_artist_artwork(wanted).await {
                tracing::debug!(%error, "artist artwork refresh failed");
            }
        });
    });
}

/// The tiles in name order: every listed artist but a bare joined credit whose primary has one.
pub fn grid_artists(artists: &[api::ArtistInfo]) -> Vec<api::ArtistInfo> {
    let names: std::collections::HashSet<String> = artists
        .iter()
        .map(|artist| normalize_artist_key(&artist.name))
        .collect();
    let mut shown: Vec<api::ArtistInfo> = artists
        .iter()
        .filter(|artist| !artist.name.trim().is_empty())
        .filter(|artist| {
            artist.id.is_some()
                || !joined_credit_primary(&normalize_artist_key(&artist.name))
                    .is_some_and(|primary| names.contains(primary))
        })
        .cloned()
        .collect();
    shown.sort_by_cached_key(|artist| (artist.name.to_lowercase(), artist.id.clone()));
    shown
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artist(name: &str, id: Option<&str>) -> api::ArtistInfo {
        api::ArtistInfo {
            name: name.into(),
            id: id.map(Into::into),
            ..Default::default()
        }
    }

    #[test]
    fn the_grid_drops_bare_joins_and_keeps_homonyms_apart() {
        let listed = [
            artist("Zebra", None),
            artist("COOL&CREATE, beatMARIO", None),
            artist("COOL&CREATE", None),
            artist("Ada", Some("ar-2")),
            artist("Ada", Some("ar-1")),
            artist("  ", None),
        ];

        let shown: Vec<(String, Option<String>)> = grid_artists(&listed)
            .into_iter()
            .map(|artist| (artist.name, artist.id))
            .collect();

        assert_eq!(
            shown,
            [
                ("Ada".to_string(), Some("ar-1".to_string())),
                ("Ada".to_string(), Some("ar-2".to_string())),
                ("COOL&CREATE".to_string(), None),
                ("Zebra".to_string(), None),
            ]
        );
    }
}
