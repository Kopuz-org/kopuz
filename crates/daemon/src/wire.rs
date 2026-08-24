//! Boundary conversion from the internal track model to wire rows.

use api::{TrackInfo, TrackKind};
use reader::Track;
use utils::playback_ref::PlaybackItemRef;

pub(crate) fn urlencode(value: &str) -> String {
    percent_encoding::utf8_percent_encode(value, percent_encoding::NON_ALPHANUMERIC).to_string()
}

pub(crate) fn artwork_url(key: &str) -> String {
    format!("/v1/artwork?track={}", urlencode(key))
}

/// The wire row for a track: sentinel durations become an explicit kind,
/// artwork becomes an entity-addressed daemon URL, and the offline flag is
/// derived from the config's registration map so clients never see paths.
pub(crate) fn track_info(track: &Track, config: &config::AppConfig) -> TrackInfo {
    let key = track.id.key().to_string();
    let uid = track.id.uid();
    let item_ref = PlaybackItemRef::parse(&uid);
    let radio = track.duration == u64::MAX;
    let offline = item_ref
        .primary_id()
        .is_some_and(|id| config.offline_tracks.contains_key(id));
    TrackInfo {
        artwork: (!radio).then(|| artwork_url(&key)),
        key,
        uid,
        title: track.title.clone(),
        artist: track.artist.clone(),
        album: track.album.clone(),
        album_id: track.album_id.clone(),
        duration_ms: (!radio).then(|| track.duration.saturating_mul(1000)),
        khz: track.khz,
        bitrate: track.bitrate,
        track_number: track.track_number,
        disc_number: track.disc_number,
        kind: if radio {
            TrackKind::Radio
        } else {
            TrackKind::Normal
        },
        seekable: !radio,
        offline,
    }
}
