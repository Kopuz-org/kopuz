use std::collections::BTreeMap;
use std::path::PathBuf;

use async_trait::async_trait;
use config::{MusicService, Source};
use db::Db;

use crate::clippsly::ClippslyClient;
use crate::server_ops::ServerConn;

use super::{
    AlbumType, ArtistView, AuthOutcome, Capabilities, FavoritesSync, LibrarySnapshot, MediaSource,
    PlaylistOps, RadioSeeds, SourceError, StreamInfo,
};

pub(super) struct ClippslySource {
    db: Db,
    source: Source,
    client: Result<ClippslyClient, SourceError>,
}

impl ClippslySource {
    pub(super) fn new(db: Db, source: Source, conn: &ServerConn) -> Self {
        Self {
            db,
            source,
            client: ClippslyClient::new(&conn.url, &conn.token),
        }
    }

    fn client(&self) -> Result<&ClippslyClient, SourceError> {
        self.client.as_ref().map_err(Clone::clone)
    }

    fn track(&self, row: crate::clippsly::Track) -> reader::Track {
        let cover = row
            .cover_art
            .as_deref()
            .and_then(|url| self.client().ok()?.media_url(url));
        let album_id = row
            .album_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| format!("track-{}", row.id));
        reader::Track {
            id: reader::TrackId::Server {
                service: MusicService::Clippsly,
                item_id: row.id.to_string(),
            },
            cover,
            album_id: reader::CoverRef::stored_item_ref(MusicService::Clippsly, &album_id, None),
            title: row.title,
            artist: row.artist.clone(),
            album: row.album_title.unwrap_or_default(),
            duration: row.duration_seconds.unwrap_or_default().max(0.0) as u64,
            khz: 0,
            bitrate: 0,
            track_number: row.track_position,
            disc_number: None,
            musicbrainz_release_id: None,
            musicbrainz_recording_id: None,
            musicbrainz_track_id: None,
            playlist_item_id: None,
            artists: vec![row.artist.clone()],
            credits: vec![reader::ArtistCredit::unlinked(row.artist)],
            replay_gain: Default::default(),
        }
    }
}

#[async_trait]
impl MediaSource for ClippslySource {
    fn source(&self) -> &Source {
        &self.source
    }
    fn db(&self) -> &Db {
        &self.db
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            edit_tags: false,
            delete_from_disk: false,
            scan_folders: false,
            folders: false,
            browse_folders: false,
            external_devices: false,
            browser_playback: false,
            sync: true,
            downloads: true,
            uploads: true,
            storage_quota: true,
            discover: false,
            dont_recommend: false,
            radio: RadioSeeds::NONE,
            playlists: PlaylistOps::None,
            artist_view: ArtistView::Library,
            albums: AlbumType::Standard,
            favorites_sync: FavoritesSync::Instant,
        }
    }

    async fn fetch_library(&self) -> Result<LibrarySnapshot, SourceError> {
        let rows = self.client()?.tracks().await?;
        let mut albums = BTreeMap::new();
        let mut tracks = Vec::with_capacity(rows.len());
        for row in rows {
            let genre = row.genre.clone().unwrap_or_default();
            let track = self.track(row);
            albums
                .entry(track.album_id.clone())
                .or_insert_with(|| reader::Album {
                    id: track.album_id.clone(),
                    title: track.album.clone(),
                    artist: track.artist.clone(),
                    genre,
                    year: 0,
                    cover_path: track.cover.as_ref().map(PathBuf::from),
                    manual_cover: false,
                    artist_id: None,
                    artist_key: None,
                });
            tracks.push(track);
        }
        Ok(LibrarySnapshot {
            albums: albums.into_values().collect(),
            tracks,
            artist_images: Vec::new(),
        })
    }

    async fn resolve_stream(&self, id: &str) -> Result<StreamInfo, SourceError> {
        let client = self.client()?;
        let stream = client.stream(id).await?;
        Ok(StreamInfo {
            url: client.stream_url(&stream)?,
            format: None,
            user_agent: None,
            duration_secs: stream.duration_seconds.map(|value| value.max(0.0) as u64),
            bitrate: stream.bitrate,
            content_length: None,
        })
    }

    async fn validate(&self) -> AuthOutcome {
        let Ok(client) = self.client() else {
            return AuthOutcome::Unreachable;
        };
        match client.account().await {
            Ok(_) => AuthOutcome::Valid,
            Err(SourceError::Auth) => AuthOutcome::Expired,
            Err(_) => AuthOutcome::Unreachable,
        }
    }

    async fn storage_quota(&self) -> Result<super::StorageQuota, SourceError> {
        let quota = self.client()?.storage().await?;
        Ok(super::StorageQuota {
            used_bytes: quota.used_bytes,
            quota_bytes: quota.quota_bytes,
            remaining_bytes: quota.quota_remaining_bytes,
        })
    }

    async fn upload_track(&self, filename: String, content: Vec<u8>) -> Result<(), SourceError> {
        self.client()?.upload(filename, content).await
    }

    async fn fetch_favorites(&self) -> Result<Vec<String>, SourceError> {
        self.db
            .favorites(self.source.as_str())
            .await
            .map_err(SourceError::from)
    }

    async fn push_favorite(&self, _: &str, _: bool) -> Result<(), SourceError> {
        Ok(())
    }

    async fn add_to_playlist(&self, _: &str, _: &[String]) -> Result<Vec<String>, SourceError> {
        Err(SourceError::unsupported("playlists"))
    }

    async fn create_playlist(&self, _: &str, _: &[String]) -> Result<String, SourceError> {
        Err(SourceError::unsupported("playlists"))
    }

    async fn remove_from_playlist(
        &self,
        _: &str,
        _: &reader::Track,
        _: usize,
    ) -> Result<(), SourceError> {
        Err(SourceError::unsupported("playlists"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn clippsly_tracks_keep_identity_and_artwork_when_stored() {
        let dir = tempfile::tempdir().unwrap();
        let db = db::init(&dir.path().join("library.db")).await.unwrap();
        let source = ClippslySource {
            db: db.clone(),
            source: Source::Server("locker".into()),
            client: ClippslyClient::new("https://listener.example.test", "test-password"),
        };
        let row = serde_json::from_value(serde_json::json!({
            "id": 42, "title": "Song", "artist": "Artist", "album_id": 7,
            "album_title": "Album", "cover_art": "/covers/7.jpg", "duration_seconds": 12.5,
            "track_position": 3
        }))
        .unwrap();
        let track = source.track(row);
        assert_eq!(track.id.uid(), "clippsly:42");
        assert_eq!(track.album_id, "clippsly:7");
        assert_eq!(track.duration, 12);
        assert_eq!(track.track_number, Some(3));
        assert_eq!(
            reader::CoverRef::for_track(&track),
            reader::CoverRef::EmbeddedUrl("https://listener.example.test/covers/7.jpg".into())
        );
        source
            .upsert_tracks(std::slice::from_ref(&track))
            .await
            .unwrap();
        let stored = db
            .tracks_by_keys(source.source(), &["42".into()])
            .await
            .unwrap();
        assert_eq!(stored[0].id, track.id);
        assert_eq!(stored[0].cover, track.cover);
        let caps = source.capabilities();
        assert!(caps.sync && caps.downloads && caps.uploads && caps.storage_quota);
        assert_eq!(caps.playlists, PlaylistOps::None);
    }
}
