//! Clippsly, a music locker, as a kopuz plugin.
//!
//! The library is the person's uploads. Clippsly keeps no favorites or
//! playlists, so the plugin leaves both to kopuz.

mod client;

use std::collections::BTreeMap;

use client::ClippslyClient;
use kopuz_plugin_sdk::{
    Album, ArtistCredit, AuthOutcome, AuthSession, Connection, LibrarySnapshot, Result, Source,
    SourceError, StorageQuota, StreamInfo, StreamQuality, Track, cover,
};

const SERVICE: &str = "clippsly";

pub struct Clippsly {
    /// A malformed server URL is only reported when something is asked of it.
    client: Result<ClippslyClient>,
}

kopuz_plugin_sdk::export_source!(Clippsly);

impl Clippsly {
    fn client(&self) -> Result<&ClippslyClient> {
        self.client.as_ref().map_err(Clone::clone)
    }

    fn track(&self, row: client::Track) -> Track {
        let cover = row
            .cover_art
            .as_deref()
            .and_then(|url| self.client().ok()?.media_url(url));
        let album_id = row
            .album_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| format!("track-{}", row.id));
        Track {
            item_id: row.id.to_string(),
            cover,
            album_id: cover::item_ref(SERVICE, &album_id, None),
            title: row.title,
            artist: row.artist.clone(),
            album: row.album_title.unwrap_or_default(),
            duration_secs: row.duration_seconds.unwrap_or_default().max(0.0) as u64,
            track_number: row.track_position,
            artists: vec![row.artist.clone()],
            credits: vec![ArtistCredit::unlinked(row.artist)],
            ..Track::default()
        }
    }
}

impl Source for Clippsly {
    /// An app password signs in on its own; the account it belongs to is the user id.
    fn login(connection: Connection, _: &str, password: &str) -> Result<AuthSession> {
        let account = ClippslyClient::new(&connection.url, password)?.account()?;
        Ok(AuthSession {
            access_token: password.to_string(),
            user_id: account.id.to_string(),
        })
    }

    fn open(connection: Connection) -> Self {
        Self {
            client: ClippslyClient::new(&connection.url, &connection.token),
        }
    }

    fn validate(&self) -> AuthOutcome {
        let Ok(client) = self.client() else {
            return AuthOutcome::Unreachable;
        };
        match client.account() {
            Ok(_) => AuthOutcome::Valid,
            Err(SourceError::Auth) => AuthOutcome::Expired,
            Err(_) => AuthOutcome::Unreachable,
        }
    }

    fn fetch_library(&self) -> Result<LibrarySnapshot> {
        let rows = self.client()?.tracks()?;
        let mut albums = BTreeMap::new();
        let mut tracks = Vec::with_capacity(rows.len());
        for row in rows {
            let genre = row.genre.clone().unwrap_or_default();
            let track = self.track(row);
            albums
                .entry(track.album_id.clone())
                .or_insert_with(|| Album {
                    id: track.album_id.clone(),
                    title: track.album.clone(),
                    artist: track.artist.clone(),
                    genre,
                    year: 0,
                    cover: track.cover.clone(),
                    artist_id: None,
                });
            tracks.push(track);
        }
        Ok(LibrarySnapshot {
            albums: albums.into_values().collect(),
            tracks,
            artists: Vec::new(),
        })
    }

    fn resolve_stream(&self, item_id: &str, _: StreamQuality) -> Result<StreamInfo> {
        let client = self.client()?;
        let stream = client.stream(item_id)?;
        Ok(StreamInfo {
            duration_secs: stream.duration_seconds.map(|value| value.max(0.0) as u64),
            bitrate: stream.bitrate,
            ..StreamInfo::url(client.stream_url(&stream)?)
        })
    }

    fn storage_quota(&self) -> Result<StorageQuota> {
        let quota = self.client()?.storage()?;
        Ok(StorageQuota {
            used_bytes: quota.used_bytes,
            quota_bytes: quota.quota_bytes,
            remaining_bytes: quota.quota_remaining_bytes,
        })
    }

    fn upload_track(&self, filename: &str, content: &[u8]) -> Result<()> {
        self.client()?.upload(filename, content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clippsly_tracks_keep_identity_and_artwork() {
        let source = Clippsly {
            client: ClippslyClient::new("https://listener.example.test", "test-password"),
        };
        let row = serde_json::from_value(serde_json::json!({
            "id": 42, "title": "Song", "artist": "Artist", "album_id": 7,
            "album_title": "Album", "cover_art": "/covers/7.jpg", "duration_seconds": 12.5,
            "track_position": 3
        }))
        .unwrap();
        let track = source.track(row);
        assert_eq!(track.item_id, "42");
        assert_eq!(track.album_id, "clippsly:7");
        assert_eq!(track.duration_secs, 12);
        assert_eq!(track.track_number, Some(3));
        assert_eq!(
            track.cover.as_deref(),
            Some("https://listener.example.test/covers/7.jpg")
        );
    }

    #[test]
    fn a_track_without_an_album_is_its_own() {
        let source = Clippsly {
            client: ClippslyClient::new("https://listener.example.test", "p"),
        };
        let row = serde_json::from_value(serde_json::json!({
            "id": 9, "title": "Single", "artist": "Artist"
        }))
        .unwrap();
        assert_eq!(source.track(row).album_id, "clippsly:track-9");
    }
}
