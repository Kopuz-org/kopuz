use async_trait::async_trait;
use config::Source;
use db::Db;

use super::{
    AlbumType, ArtistView, AuthOutcome, Capabilities, FavoritesSync, MediaSource, PlaylistOps,
    SourceError, StreamInfo,
};

pub(super) struct OfflineServerSource {
    pub(super) db: Db,
    pub(super) source: Source,
}

#[async_trait]
impl MediaSource for OfflineServerSource {
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
            sync: false,
            downloads: false,
            discover: false,
            radio: false,
            playlists: PlaylistOps::None,
            artist_view: ArtistView::Library,
            albums: AlbumType::Standard,
            favorites_sync: FavoritesSync::Instant,
        }
    }

    async fn add_to_playlist(&self, _: &str, _: &[String]) -> Result<Vec<String>, SourceError> {
        Err(SourceError::Connectivity)
    }
    async fn create_playlist(&self, _: &str, _: &[String]) -> Result<String, SourceError> {
        Err(SourceError::Connectivity)
    }
    async fn remove_from_playlist(
        &self,
        _: &str,
        _: &reader::Track,
        _: usize,
    ) -> Result<(), SourceError> {
        Err(SourceError::Connectivity)
    }
    async fn resolve_stream(&self, _: &str) -> Result<StreamInfo, SourceError> {
        Err(SourceError::Auth)
    }
    async fn validate(&self) -> AuthOutcome {
        AuthOutcome::Unreachable
    }
    async fn fetch_favorites(&self) -> Result<Vec<String>, SourceError> {
        Err(SourceError::Connectivity)
    }
    async fn push_favorite(&self, _: &str, _: bool) -> Result<(), SourceError> {
        Err(SourceError::Connectivity)
    }
}
