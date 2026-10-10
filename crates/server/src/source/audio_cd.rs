use async_trait::async_trait;
use config::{MusicService, Source};
use db::Db;

use super::{
    AlbumType, ArtistView, AuthOutcome, Capabilities, FavoritesSync, LibrarySnapshot, MediaSource,
    PlaylistOps, RadioSeeds, SourceError, StreamInfo,
};

pub(super) struct AudioCdSource {
    db: Db,
    source: Source,
    device: String,
    reader: crate::audio_cd::Reader,
}

impl AudioCdSource {
    pub(super) fn new(db: Db, source: Source, device: &str) -> Self {
        Self {
            db,
            source,
            device: device.to_string(),
            reader: crate::audio_cd::Reader::new(device),
        }
    }
}

#[async_trait]
impl MediaSource for AudioCdSource {
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
            sync: false,
            downloads: false,
            uploads: false,
            storage_quota: false,
            discover: false,
            dont_recommend: false,
            radio: RadioSeeds::NONE,
            playlists: PlaylistOps::None,
            artist_view: ArtistView::Library,
            albums: AlbumType::Standard,
            favorites_sync: FavoritesSync::Instant,
        }
    }

    fn rip_audio(&self) -> bool {
        crate::audio_cd::supported()
    }

    async fn fetch_library(&self) -> Result<LibrarySnapshot, SourceError> {
        let device = self.device.clone();
        let disc = tokio::task::spawn_blocking(move || crate::audio_cd::inspect(&device))
            .await
            .map_err(|error| SourceError::Backend(error.to_string()))?
            .map_err(|error| SourceError::Backend(error.to_string()))?;
        Ok(snapshot(disc))
    }

    async fn open_stream(
        &self,
        item_id: &str,
        _: Option<crate::stream::stream_buffer::BufferProgressCallback>,
    ) -> Result<Option<Box<dyn symphonia::core::io::MediaSource>>, SourceError> {
        let request = self.reader.request();
        let key = item_id.to_string();
        tokio::task::spawn_blocking(move || request.open(&key))
            .await
            .map_err(|error| SourceError::Backend(error.to_string()))?
            .map(Some)
            .map_err(|error| SourceError::Backend(error.to_string()))
    }

    async fn resolve_stream(&self, _: &str) -> Result<StreamInfo, SourceError> {
        Err(SourceError::unsupported("CD stream URLs"))
    }

    async fn validate(&self) -> AuthOutcome {
        match self.fetch_library().await {
            Ok(_) => AuthOutcome::Valid,
            Err(_) => AuthOutcome::Unreachable,
        }
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
        Err(SourceError::unsupported("CD playlists"))
    }
    async fn create_playlist(&self, _: &str, _: &[String]) -> Result<String, SourceError> {
        Err(SourceError::unsupported("CD playlists"))
    }
    async fn remove_from_playlist(
        &self,
        _: &str,
        _: &reader::Track,
        _: usize,
    ) -> Result<(), SourceError> {
        Err(SourceError::unsupported("CD playlists"))
    }
}

pub(super) fn snapshot(disc: crate::audio_cd::Disc) -> LibrarySnapshot {
    let album_id = format!("cdda:{}", disc.id);
    let title = format!("Audio CD ({})", &disc.id[..8]);
    let tracks = disc
        .tracks
        .iter()
        .filter(|t| t.audio)
        .map(|t| reader::Track {
            id: reader::TrackId::Server {
                service: MusicService::AudioCd,
                item_id: disc.key(t),
            },
            cover: Some(reader::CoverRef::NO_COVER.to_string()),
            album_id: album_id.clone(),
            title: format!("Track {:02}", t.number),
            artist: String::new(),
            album: title.clone(),
            duration: (t.end - t.start) as u64 / 75,
            khz: 44100,
            bitrate: 1411,
            track_number: Some(u32::from(t.number)),
            disc_number: Some(1),
            musicbrainz_release_id: None,
            musicbrainz_recording_id: None,
            musicbrainz_track_id: None,
            playlist_item_id: None,
            artists: Vec::new(),
            credits: Vec::new(),
            replay_gain: Default::default(),
        })
        .collect();
    LibrarySnapshot {
        albums: vec![reader::Album {
            id: album_id,
            title,
            artist: String::new(),
            genre: String::new(),
            year: 0,
            cover_path: None,
            manual_cover: false,
            artist_id: None,
            artist_key: None,
        }],
        tracks,
        artist_images: Vec::new(),
    }
}
