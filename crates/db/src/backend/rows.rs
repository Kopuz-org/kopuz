//! sqlx `FromRow` → model mappers (issue #347, step 6). The reverse of the
//! importer's column writes: rebuild the typed `TrackId`/cover from `source` +
//! `track_key` + `service` + `cover_path`.

use std::path::PathBuf;

use reader::models::{Album, ArtistCredit, Track, TrackId};

#[derive(sqlx::FromRow)]
pub struct TrackRow {
    pub rowid_pk: i64,
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
    pub rg_track_gain: Option<f64>,
    pub rg_track_peak: Option<f64>,
    pub rg_album_gain: Option<f64>,
    pub rg_album_peak: Option<f64>,
}

/// One credit of a track, with the artist row it is filed under.
#[derive(sqlx::FromRow)]
pub struct CreditRow {
    pub track_pk: i64,
    pub name: String,
    pub artist_key: String,
    pub source: String,
    pub source_artist_id: Option<String>,
}

impl From<CreditRow> for ArtistCredit {
    fn from(r: CreditRow) -> Self {
        ArtistCredit {
            name: r.name,
            id: r.source_artist_id,
            source: Some(config::Source::from_column(&r.source)),
            key: Some(r.artist_key),
        }
    }
}

impl TrackRow {
    pub fn into_track(self, credits: Vec<ArtistCredit>) -> Track {
        let id = track_id(self.service.as_deref(), self.track_key);
        Track {
            id,
            cover: self.cover_path,
            album_id: self.source_album_id,
            title: self.title,
            artist: self.artist,
            album: self.album,
            duration: self.duration.max(0) as u64,
            khz: self.khz.max(0) as u32,
            bitrate: self.bitrate.clamp(0, u16::MAX as i64) as u16,
            track_number: self.track_number.map(|n| n as u32),
            disc_number: self.disc_number.map(|n| n as u32),
            musicbrainz_release_id: self.mb_release_id,
            musicbrainz_recording_id: self.mb_recording_id,
            musicbrainz_track_id: self.mb_track_id,
            playlist_item_id: None,
            artists: credits.iter().map(|credit| credit.name.clone()).collect(),
            credits,
            replay_gain: config::ReplayGainInfo {
                track_gain_db: self.rg_track_gain.map(|v| v as f32),
                track_peak: self.rg_track_peak.map(|v| v as f32),
                album_gain_db: self.rg_album_gain.map(|v| v as f32),
                album_peak: self.rg_album_peak.map(|v| v as f32),
            },
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
    pub artist_key: Option<String>,
    pub artist_source_id: Option<String>,
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
            artist_id: r.artist_source_id,
            artist_key: r.artist_key,
        }
    }
}

/// A stored row's id: a local path when no service issued it, else that service's item.
fn track_id(service: Option<&str>, track_key: String) -> TrackId {
    match service {
        None => TrackId::Local(PathBuf::from(track_key)),
        Some(service) => TrackId::Server {
            service: parse_service(service),
            item_id: track_key,
        },
    }
}

#[derive(sqlx::FromRow)]
pub struct QueueTrackRow {
    pub position: i64,
    pub track_key: String,
    pub service: Option<String>,
    pub source_album_id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: Option<i64>,
    pub khz: i64,
    pub bitrate: i64,
    pub track_number: Option<i64>,
    pub disc_number: Option<i64>,
    pub cover_path: Option<String>,
    pub mb_release_id: Option<String>,
    pub mb_recording_id: Option<String>,
    pub mb_track_id: Option<String>,
    pub playlist_item_id: Option<String>,
    pub rg_track_gain: Option<f64>,
    pub rg_track_peak: Option<f64>,
    pub rg_album_gain: Option<f64>,
    pub rg_album_peak: Option<f64>,
}

impl QueueTrackRow {
    pub fn into_track(self, credits: Vec<ArtistCredit>) -> Track {
        Track {
            id: track_id(self.service.as_deref(), self.track_key),
            cover: self.cover_path,
            album_id: self.source_album_id,
            title: self.title,
            artist: self.artist,
            album: self.album,
            duration: self.duration.map_or(u64::MAX, |secs| secs.max(0) as u64),
            khz: self.khz.max(0) as u32,
            bitrate: self.bitrate.clamp(0, u16::MAX as i64) as u16,
            track_number: self.track_number.map(|n| n as u32),
            disc_number: self.disc_number.map(|n| n as u32),
            musicbrainz_release_id: self.mb_release_id,
            musicbrainz_recording_id: self.mb_recording_id,
            musicbrainz_track_id: self.mb_track_id,
            playlist_item_id: self.playlist_item_id,
            artists: credits.iter().map(|credit| credit.name.clone()).collect(),
            credits,
            replay_gain: config::ReplayGainInfo {
                track_gain_db: self.rg_track_gain.map(|v| v as f32),
                track_peak: self.rg_track_peak.map(|v| v as f32),
                album_gain_db: self.rg_album_gain.map(|v| v as f32),
                album_peak: self.rg_album_peak.map(|v| v as f32),
            },
        }
    }
}

#[derive(sqlx::FromRow)]
pub struct QueueCreditRow {
    pub queue_position: i64,
    pub name: String,
    pub source_artist_id: Option<String>,
    pub source: Option<String>,
    pub artist_key: Option<String>,
}

impl From<QueueCreditRow> for ArtistCredit {
    fn from(r: QueueCreditRow) -> Self {
        ArtistCredit {
            name: r.name,
            key: r.artist_key,
            id: r.source_artist_id,
            source: r.source.as_deref().map(config::Source::from_column),
        }
    }
}

/// The inverse of `service_str`. Every variant needs an arm: a missing one
/// reads that service's tracks back as Jellyfin, and their covers then resolve
/// against the wrong server.
pub fn parse_service(s: &str) -> config::MusicService {
    match s {
        "Subsonic" => config::MusicService::Subsonic,
        "Custom" => config::MusicService::Custom,
        "YtMusic" => config::MusicService::YtMusic,
        "SoundCloud" => config::MusicService::SoundCloud,
        "AppleMusic" => config::MusicService::AppleMusic,
        "Spotify" => config::MusicService::Spotify,
        "Nextcloud" => config::MusicService::Nextcloud,
        _ => config::MusicService::Jellyfin,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_service_round_trips_through_the_column() {
        use config::MusicService::*;

        for service in [
            Jellyfin, Subsonic, Custom, YtMusic, AppleMusic, SoundCloud, Spotify, Nextcloud,
        ] {
            let stored = crate::backend::writes::service_str(service);
            assert_eq!(parse_service(stored), service, "{stored} did not survive");
        }
    }
}
