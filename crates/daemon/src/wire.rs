//! Boundary conversion from the internal track model to wire rows.

use api::{TrackInfo, TrackKind};
use reader::Track;

/// The wire row for a track: sentinel durations become an explicit kind,
/// artwork becomes a presence marker carrying the entity key (clients fetch
/// the bytes via the GetArtwork rpc), and the offline flag is derived from
/// the config's registration map so clients never see paths.
pub(crate) fn track_info(track: &Track, config: &config::AppConfig) -> TrackInfo {
    let key = track.id.key().to_string();
    let uid = track.id.uid();
    let radio = track.duration == u64::MAX;
    let offline = match &track.id {
        reader::TrackId::Server { item_id, .. } => config.offline_tracks.contains_key(item_id),
        reader::TrackId::Local(_) => false,
    };
    TrackInfo {
        artwork: (!radio).then(|| key.clone()),
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
        service: track.id.service().map(music_service),
        artists: track.artists.clone(),
        musicbrainz_release_id: track.musicbrainz_release_id.clone(),
        musicbrainz_recording_id: track.musicbrainz_recording_id.clone(),
        musicbrainz_track_id: track.musicbrainz_track_id.clone(),
        playlist_item_id: track.playlist_item_id.clone(),
        source: config.active_source.as_str().to_string(),
    }
}

pub(crate) fn music_service(value: config::MusicService) -> api::MusicService {
    match value {
        config::MusicService::Jellyfin => api::MusicService::Jellyfin,
        config::MusicService::Subsonic => api::MusicService::Subsonic,
        config::MusicService::Custom => api::MusicService::Custom,
        config::MusicService::YtMusic => api::MusicService::YtMusic,
        config::MusicService::AppleMusic => api::MusicService::AppleMusic,
        config::MusicService::SoundCloud => api::MusicService::SoundCloud,
        config::MusicService::Spotify => api::MusicService::Spotify,
        config::MusicService::Nextcloud => api::MusicService::Nextcloud,
    }
}

pub(crate) fn config_music_service(value: api::MusicService) -> Option<config::MusicService> {
    Some(match value {
        api::MusicService::Jellyfin => config::MusicService::Jellyfin,
        api::MusicService::Subsonic => config::MusicService::Subsonic,
        api::MusicService::Custom => config::MusicService::Custom,
        api::MusicService::YtMusic => config::MusicService::YtMusic,
        api::MusicService::AppleMusic => config::MusicService::AppleMusic,
        api::MusicService::SoundCloud => config::MusicService::SoundCloud,
        api::MusicService::Spotify => config::MusicService::Spotify,
        api::MusicService::Nextcloud => config::MusicService::Nextcloud,
        api::MusicService::Unknown => return None,
    })
}
