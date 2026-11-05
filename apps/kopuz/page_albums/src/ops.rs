//! The album commands a card menu or a detail button issues.

use api::TrackInfo;
use gpui_kit::App;
use store::actions;

/// Album order: by disc, then by track number.
pub fn in_album_order(tracks: &mut [TrackInfo]) {
    tracks.sort_by(|left, right| {
        left.disc_number
            .unwrap_or(1)
            .cmp(&right.disc_number.unwrap_or(1))
            .then_with(|| {
                left.track_number
                    .unwrap_or(0)
                    .cmp(&right.track_number.unwrap_or(0))
            })
    });
}

pub fn keys_of(tracks: &[TrackInfo]) -> Vec<String> {
    tracks
        .iter()
        .filter(|track| !track.key.is_empty())
        .map(|track| track.key.clone())
        .collect()
}

pub fn delete_tracks(keys: Vec<String>, cx: &mut App) {
    actions::run(
        cx,
        move |api| async move { api.delete_tracks(keys, true).await },
    );
}
