//! Playlists and folders.
//!
//! Every mutation goes through the active source, so a server playlist is
//! pushed to the server and a local one is only written locally -- the caller
//! never branches on which it holds. Each one invalidates the tables it
//! touched, which is how a frontend learns to re-read.
//!
//! The per-service identity of a playlist entry differs (a video id, an entry
//! id, a position), so the source trait takes the whole track for a removal or
//! a reorder. Resolving a key to that track is this service's job, not a
//! caller's.

use std::sync::Arc;

use api::{
    ApiError, ArtworkTarget, PlaylistCatalog, PlaylistFolderInfo, PlaylistInfo, PlaylistReorder,
    Table,
};

use crate::session::SessionHandle;

pub struct PlaylistService {
    db: db::Db,
    session: SessionHandle,
}

fn source_error(error: server::source::SourceError) -> ApiError {
    use api::ErrorCode;
    use server::source::SourceError;
    match &error {
        SourceError::Unsupported(what) => ApiError::unsupported(*what),
        SourceError::Auth => ApiError::new(ErrorCode::SourceAuthExpired, error.to_string()),
        SourceError::Connectivity => ApiError::new(ErrorCode::SourceUnreachable, error.to_string()),
        SourceError::InvalidInput(message) => ApiError::invalid_input(message.clone()),
        SourceError::Backend(message) => ApiError::internal(message.clone()),
    }
}

fn db_error(error: db::DbError) -> ApiError {
    ApiError::internal(format!("database error: {error}"))
}

impl PlaylistService {
    pub fn new(db: db::Db, session: SessionHandle) -> Arc<Self> {
        Arc::new(Self { db, session })
    }

    fn config(&self) -> config::AppConfig {
        self.session.config_watch().borrow().clone()
    }

    fn active_source(&self) -> server::source::ActiveSource {
        Arc::from(server::source::active(self.db.clone(), &self.config()))
    }

    pub async fn catalog(&self) -> Result<PlaylistCatalog, ApiError> {
        let store = self
            .db
            .load_playlists(&self.config().active_source)
            .await
            .map_err(db_error)?;
        Ok(PlaylistCatalog {
            playlists: store
                .playlists
                .into_iter()
                .map(|playlist| PlaylistInfo {
                    artwork: playlist
                        .cover_path
                        .is_some()
                        .then(|| ArtworkTarget::Album(playlist.id.clone())),
                    id: playlist.id,
                    name: playlist.name,
                    track_keys: playlist.tracks,
                })
                .collect(),
            folders: store
                .folders
                .into_iter()
                .map(|folder| PlaylistFolderInfo {
                    id: folder.id,
                    name: folder.name,
                    playlist_ids: folder.playlist_ids,
                })
                .collect(),
        })
    }

    /// The track behind one entry of a playlist, which the source needs to
    /// identify that entry on its own terms.
    async fn entry_track(
        &self,
        playlist_id: &str,
        index: usize,
    ) -> Result<reader::Track, ApiError> {
        let source = self.config().active_source;
        let store = self.db.load_playlists(&source).await.map_err(db_error)?;
        let key = store
            .playlists
            .iter()
            .find(|playlist| playlist.id == playlist_id)
            .and_then(|playlist| playlist.tracks.get(index))
            .ok_or_else(|| ApiError::not_found("no such playlist entry"))?
            .clone();
        self.db
            .tracks_by_keys(&source, &[key])
            .await
            .map_err(db_error)?
            .into_iter()
            .next()
            .ok_or_else(|| ApiError::not_found("the playlist entry names an unknown track"))
    }

    async fn entries(&self, playlist_id: &str) -> Result<Vec<String>, ApiError> {
        let store = self
            .db
            .load_playlists(&self.config().active_source)
            .await
            .map_err(db_error)?;
        store
            .playlists
            .into_iter()
            .find(|playlist| playlist.id == playlist_id)
            .map(|playlist| playlist.tracks)
            .ok_or_else(|| ApiError::not_found("no such playlist"))
    }

    pub async fn create(&self, name: &str, keys: &[String]) -> Result<String, ApiError> {
        let id = self
            .active_source()
            .create_playlist(name, keys)
            .await
            .map_err(source_error)?;
        self.session.invalidate(Table::Playlists);
        Ok(id)
    }

    pub async fn rename(&self, id: &str, name: &str) -> Result<(), ApiError> {
        self.active_source()
            .upsert_playlist_meta(id, name, None, None)
            .await
            .map_err(source_error)?;
        self.session.invalidate(Table::Playlists);
        Ok(())
    }

    pub async fn delete(&self, id: &str) -> Result<(), ApiError> {
        self.active_source()
            .delete_playlist(id)
            .await
            .map_err(source_error)?;
        self.session.invalidate(Table::Playlists);
        // A deleted playlist leaves every folder that held it.
        self.session.invalidate(Table::Folders);
        Ok(())
    }

    pub async fn add_tracks(&self, id: &str, keys: &[String]) -> Result<(), ApiError> {
        self.active_source()
            .add_to_playlist(id, keys)
            .await
            .map_err(source_error)?;
        self.session.invalidate(Table::Playlists);
        Ok(())
    }

    pub async fn remove_track(&self, id: &str, index: u32) -> Result<(), ApiError> {
        let index = index as usize;
        let track = self.entry_track(id, index).await?;
        self.active_source()
            .remove_from_playlist(id, &track, index)
            .await
            .map_err(source_error)?;
        self.session.invalidate(Table::Playlists);
        Ok(())
    }

    pub async fn reorder(&self, id: &str, reorder: PlaylistReorder) -> Result<(), ApiError> {
        let (from, to) = (reorder.from as usize, reorder.to as usize);
        let mut keys = self.entries(id).await?;
        if from >= keys.len() || to >= keys.len() {
            return Err(ApiError::invalid_input("playlist position out of range"));
        }
        let track = self.entry_track(id, from).await?;
        let moved = keys.remove(from);
        keys.insert(to, moved);
        self.active_source()
            .reorder_playlist(id, &keys, &track, to)
            .await
            .map_err(source_error)?;
        self.session.invalidate(Table::Playlists);
        Ok(())
    }

    pub async fn refresh(&self, id: &str) -> Result<(), ApiError> {
        let entries = self
            .active_source()
            .fetch_playlist_entries(id)
            .await
            .map_err(source_error)?;
        let keys: Vec<String> = entries
            .iter()
            .map(|track| track.id.key().to_string())
            .collect();
        let source = self.active_source();
        source.upsert_tracks(&entries).await.map_err(source_error)?;
        source
            .set_playlist_tracks(id, &keys)
            .await
            .map_err(source_error)?;
        self.session.invalidate(Table::Playlists);
        self.session.invalidate(Table::Tracks);
        Ok(())
    }

    pub async fn create_folder(&self, name: &str) -> Result<String, ApiError> {
        let id = uuid::Uuid::new_v4().to_string();
        self.active_source()
            .create_folder(&id, name)
            .await
            .map_err(source_error)?;
        self.session.invalidate(Table::Folders);
        Ok(id)
    }

    pub async fn rename_folder(&self, id: &str, name: &str) -> Result<(), ApiError> {
        self.active_source()
            .rename_folder(id, name)
            .await
            .map_err(source_error)?;
        self.session.invalidate(Table::Folders);
        Ok(())
    }

    pub async fn delete_folder(&self, id: &str) -> Result<(), ApiError> {
        self.active_source()
            .delete_folder(id)
            .await
            .map_err(source_error)?;
        self.session.invalidate(Table::Folders);
        Ok(())
    }

    pub async fn move_playlist(
        &self,
        playlist_id: &str,
        folder_id: Option<&str>,
    ) -> Result<(), ApiError> {
        self.active_source()
            .set_playlist_folder(playlist_id, folder_id)
            .await
            .map_err(source_error)?;
        self.session.invalidate(Table::Folders);
        Ok(())
    }
}
