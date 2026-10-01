//! Asking the daemon for photos of the artists a grid shows; it stores and announces what it finds.

use dioxus::prelude::*;

/// Ask the daemon to fill in missing artist photos for what this grid renders.
pub fn use_artist_photo_fetch(artists: Resource<Vec<api::ArtistInfo>>) {
    // Each photo found dirties `artists`, so an ask not remembered would repeat forever.
    let mut asked_for = use_signal(Vec::<String>::new);
    use_effect(move || {
        let wanted: Vec<String> = artists
            .read()
            .iter()
            .flatten()
            .map(|artist| artist.key.clone())
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
