//! sqlx `FromRow` → model mappers (issue #347, step 6). The reverse of the
//! importer's column writes: rebuild the typed `TrackId`/cover from `source` +
//! `track_key` + `service` + `cover_path`.

use std::path::PathBuf;

use reader::models::{Album, Track, TrackId};

#[derive(sqlx::FromRow)]
pub struct TrackRow {
    pub source: String,
    pub track_key: String,
    pub service: Option<String>,
    pub cover_path: Option<String>,
    pub source_album_id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: i64,
    pub khz: i64,
    pub bitrate: i64,
    pub track_number: Option<i64>,
    pub disc_number: Option<i64>,
    pub mb_release_id: Option<String>,
    pub mb_recording_id: Option<String>,
    pub mb_track_id: Option<String>,
    pub playlist_item_id: Option<String>,
    pub artists_json: String,
}

impl From<TrackRow> for Track {
    fn from(r: TrackRow) -> Self {
        let id = if r.source == "local" {
            TrackId::Local(PathBuf::from(&r.track_key))
        } else {
            TrackId::Server {
                service: parse_service(r.service.as_deref().unwrap_or("Jellyfin")),
                item_id: r.track_key,
            }
        };
        Track {
            id,
            cover: r.cover_path,
            album_id: r.source_album_id,
            title: r.title,
            artist: r.artist,
            album: r.album,
            duration: r.duration.max(0) as u64,
            khz: r.khz.max(0) as u32,
            bitrate: r.bitrate.clamp(0, u16::MAX as i64) as u16,
            track_number: r.track_number.map(|n| n as u32),
            disc_number: r.disc_number.map(|n| n as u32),
            musicbrainz_release_id: r.mb_release_id,
            musicbrainz_recording_id: r.mb_recording_id,
            musicbrainz_track_id: r.mb_track_id,
            playlist_item_id: r.playlist_item_id,
            artists: serde_json::from_str(&r.artists_json).unwrap_or_default(),
        }
    }
}

#[derive(sqlx::FromRow)]
pub struct AlbumRow {
    pub source_album_id: String,
    pub title: String,
    pub artist: String,
    pub genre: String,
    pub year: i64,
    pub cover_path: Option<String>,
    pub manual_cover: i64,
}

impl From<AlbumRow> for Album {
    fn from(r: AlbumRow) -> Self {
        Album {
            id: r.source_album_id,
            title: r.title,
            artist: r.artist,
            genre: r.genre,
            year: r.year.clamp(0, u16::MAX as i64) as u16,
            cover_path: r.cover_path.map(PathBuf::from),
            manual_cover: r.manual_cover != 0,
        }
    }
}

pub fn parse_service(s: &str) -> config::MusicService {
    match s {
        "Subsonic" => config::MusicService::Subsonic,
        "Custom" => config::MusicService::Custom,
        "YtMusic" => config::MusicService::YtMusic,
        "SoundCloud" => config::MusicService::SoundCloud,
        "AppleMusic" => config::MusicService::AppleMusic,
        _ => config::MusicService::Jellyfin,
    }
}
