//! Boundary conversion from the internal track model to wire rows.

use api::{TrackInfo, TrackKind};
use reader::{Track, TrackId};

/// The wire row for a track: sentinel durations become an explicit kind,
/// artwork becomes a presence marker carrying the entity key (clients fetch
/// the bytes via the GetArtwork rpc), and the offline flag is derived from
/// the config's registration map so clients never see paths.
pub(crate) fn track_info(track: &Track, config: &config::AppConfig) -> TrackInfo {
    let key = track.id.key().to_string();
    let offline = match &track.id {
        reader::TrackId::Server { item_id, .. } => config.offline_tracks.contains_key(item_id),
        reader::TrackId::Local(_) => false,
    };
    track_info_with(
        track,
        key,
        offline,
        config.active_source.as_str().to_string(),
    )
}

fn track_info_with(track: &Track, key: String, offline: bool, source: String) -> TrackInfo {
    let uid = track.id.uid();
    let radio = track.duration == u64::MAX;
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
        service: track.id.service().map(music_service_to_api),
        artists: track.artists.clone(),
        musicbrainz_release_id: track.musicbrainz_release_id.clone(),
        musicbrainz_recording_id: track.musicbrainz_recording_id.clone(),
        musicbrainz_track_id: track.musicbrainz_track_id.clone(),
        playlist_item_id: track.playlist_item_id.clone(),
        source,
    }
}

pub fn track_info_for_persistence(track: &Track) -> TrackInfo {
    track_info_with(track, track.id.key().into_owned(), false, String::new())
}

pub fn track_from_info_parts(value: &TrackInfo, id: TrackId, cover: Option<String>) -> Track {
    Track {
        id,
        cover,
        album_id: value.album_id.clone(),
        title: value.title.clone(),
        artist: value.artist.clone(),
        album: value.album.clone(),
        duration: if value.kind == TrackKind::Radio {
            u64::MAX
        } else {
            value.duration_ms.unwrap_or_default() / 1000
        },
        khz: value.khz,
        bitrate: value.bitrate,
        track_number: value.track_number,
        disc_number: value.disc_number,
        musicbrainz_release_id: value.musicbrainz_release_id.clone(),
        musicbrainz_recording_id: value.musicbrainz_recording_id.clone(),
        musicbrainz_track_id: value.musicbrainz_track_id.clone(),
        playlist_item_id: value.playlist_item_id.clone(),
        artists: value.artists.clone(),
    }
}

pub fn music_service_to_api(value: config::MusicService) -> api::MusicService {
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

pub fn music_service_from_api(value: api::MusicService) -> Option<config::MusicService> {
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
