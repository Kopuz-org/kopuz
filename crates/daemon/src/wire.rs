//! Boundary conversion from the internal track model to wire rows.

use api::{TrackInfo, TrackKind};
use reader::Track;
use server::playback_ref::PlaybackItemRef;

/// The wire row for a track: sentinel durations become an explicit kind, and
/// the offline flag is derived from the config's registration map so clients
/// never see paths.
pub(crate) fn track_info(track: &Track, config: &config::AppConfig) -> TrackInfo {
    let key = track.id.key().to_string();
    let uid = track.id.uid();
    let item_ref = PlaybackItemRef::parse(&uid);
    let radio = track.duration == u64::MAX;
    let offline = item_ref
        .primary_id()
        .is_some_and(|id| config.offline_tracks.contains_key(id));
    TrackInfo {
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
        format: track_format(track),
        artists: track.artists.clone(),
        musicbrainz_release_id: track.musicbrainz_release_id.clone(),
        musicbrainz_recording_id: track.musicbrainz_recording_id.clone(),
        musicbrainz_track_id: track.musicbrainz_track_id.clone(),
        playlist_item_id: track.playlist_item_id.clone(),
        artwork: crate::artwork::track_ref(track),
    }
}

/// The container a local file is in, for the badge a row shows. Only the
/// formats the player actually decodes are named; anything else, and every
/// track that came from a service, has none.
fn track_format(track: &Track) -> Option<String> {
    let extension = track
        .id
        .local_path()?
        .extension()?
        .to_str()?
        .to_ascii_lowercase();
    matches!(
        extension.as_str(),
        "mp3" | "flac" | "m4a" | "wav" | "ogg" | "opus" | "mp4" | "mka"
    )
    .then(|| extension.to_uppercase())
}
