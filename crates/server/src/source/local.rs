use async_trait::async_trait;
use config::Source;
use db::Db;

use super::{
    AlbumType, ArtistView, AuthOutcome, Capabilities, FavoritesSync, MediaSource, PlaylistOps,
    SourceError, StreamInfo, mirror_created,
};

pub(super) struct LocalSource {
    pub(super) db: Db,
    pub(super) source: Source,
}

#[async_trait]
impl MediaSource for LocalSource {
    fn source(&self) -> &Source {
        &self.source
    }
    fn db(&self) -> &Db {
        &self.db
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            edit_tags: true,
            delete_from_disk: true,
            scan_folders: true,
            folders: true,
            sync: false,
            downloads: false,
            discover: false,
            radio: false,
            playlists: PlaylistOps::Reorder,
            artist_view: ArtistView::Library,
            albums: AlbumType::Standard,
            favorites_sync: FavoritesSync::Instant,
        }
    }

    async fn add_to_playlist(
        &self,
        playlist_id: &str,
        item_refs: &[String],
    ) -> Result<Vec<String>, SourceError> {
        self.db
            .add_playlist_tracks(&self.source, playlist_id, item_refs)
            .await?;
        Ok(item_refs.to_vec())
    }

    async fn create_playlist(
        &self,
        name: &str,
        item_refs: &[String],
    ) -> Result<String, SourceError> {
        let id = uuid::Uuid::new_v4().to_string();
        mirror_created(&self.db, &self.source, &id, name, item_refs).await?;
        Ok(id)
    }

    async fn remove_from_playlist(
        &self,
        playlist_id: &str,
        track: &reader::Track,
        _position: usize,
    ) -> Result<(), SourceError> {
        let r = track.id.key().into_owned();
        self.db
            .remove_playlist_tracks(&self.source, playlist_id, &[r])
            .await
            .map_err(SourceError::from)
    }

    async fn reorder_playlist(
        &self,
        playlist_id: &str,
        ordered_refs: &[String],
        _moved: &reader::Track,
        _new_index: usize,
    ) -> Result<(), SourceError> {
        self.db
            .set_playlist_tracks(&self.source, playlist_id, ordered_refs)
            .await
            .map_err(SourceError::from)
    }

    async fn resolve_stream(&self, item_id: &str) -> Result<StreamInfo, SourceError> {
        Ok(StreamInfo {
            url: item_id.to_string(),
            format: None,
            user_agent: None,
            duration_secs: None,
            bitrate: None,
            content_length: None,
        })
    }

    async fn validate(&self) -> AuthOutcome {
        AuthOutcome::Valid
    }

    async fn fetch_favorites(&self) -> Result<Vec<String>, SourceError> {
        Ok(Vec::new())
    }

    async fn push_favorite(&self, _item_id: &str, _on: bool) -> Result<(), SourceError> {
        Ok(())
    }
}
