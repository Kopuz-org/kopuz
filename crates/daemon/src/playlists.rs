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

use api::{ApiError, PlaylistCatalog, PlaylistFolderInfo, PlaylistInfo, PlaylistReorder, Table};

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
        let config = self.config();
        let store = self
            .db
            .load_playlists(&config.active_source)
            .await
            .map_err(db_error)?;
        // A playlist with no cover of its own borrows its first track's, so
        // those tracks are fetched once for the whole catalog.
        let first_keys: Vec<String> = store
            .playlists
            .iter()
            .filter_map(|playlist| playlist.tracks.first().cloned())
            .collect();
        let first_tracks: std::collections::HashMap<String, reader::Track> = self
            .db
            .tracks_by_keys(&config.active_source, &first_keys)
            .await
            .map_err(db_error)?
            .into_iter()
            .map(|track| (track.id.key().into_owned(), track))
            .collect();
        Ok(PlaylistCatalog {
            playlists: store
                .playlists
                .into_iter()
                .map(|playlist| PlaylistInfo {
                    artwork: crate::artwork::playlist_ref(
                        &playlist,
                        &config,
                        playlist
                            .tracks
                            .first()
                            .and_then(|key| first_tracks.get(key)),
                    ),
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

    /// Pull one playlist's contents again, a page at a time.
    ///
    /// Each page is written and announced before the next is fetched, so a
    /// long playlist fills in as it arrives rather than appearing all at once
    /// at the end -- and it does so for every frontend watching, not just the
    /// one that asked. A staleness gate keeps revisiting a playlist free.
    pub async fn refresh(&self, id: &str) -> Result<(), ApiError> {
        let source = self.active_source();
        if source.capabilities().playlists == server::source::PlaylistOps::None {
            return Ok(());
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or_default();
        let last: u64 = self
            .db
            .meta_get("pl_pull", id)
            .await
            .ok()
            .flatten()
            .and_then(|raw| raw.parse().ok())
            .unwrap_or(0);
        if last <= now && now - last < 15 * 60 {
            return Ok(());
        }

        // One epoch for the walk: pages stamp their rows with it and the
        // closing sweep drops whatever the remote no longer lists.
        let epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as i64)
            .unwrap_or_default();
        let mut cursor: Option<String> = None;
        let mut position: i64 = 0;
        let mut completed = true;

        loop {
            let page = match source.fetch_playlist_entries_page(id, cursor.clone()).await {
                Ok(page) => page,
                Err(error) => {
                    tracing::warn!(%error, playlist = id, "playlist page fetch failed");
                    completed = false;
                    break;
                }
            };
            let next = page.next.clone();
            if page.tracks.is_empty() {
                break;
            }
            let page_refs: Vec<String> = page
                .tracks
                .iter()
                .map(|track| track.id.key().to_string())
                .filter(|key| !key.is_empty())
                .collect();
            for chunk in page.tracks.chunks(100) {
                let _ = source.upsert_tracks(chunk).await;
            }
            let _ = source
                .upsert_playlist_tracks_page(id, &page_refs, position, epoch)
                .await;
            position += page_refs.len() as i64;
            self.session.invalidate(Table::Tracks);
            self.session.invalidate(Table::Playlists);
            match next {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }

        if completed {
            let _ = source.sweep_playlist_tracks(id, epoch).await;
            let _ = source.set_meta("pl_pull", id, &now.to_string()).await;
            self.session.invalidate(Table::Playlists);
            self.session.invalidate(Table::Tracks);
        }
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

impl PlaylistService {
    /// Pull the server's playlists and their contents.
    ///
    /// Listing first so the tiles appear, then entries per playlist, then a
    /// full-replace: a playlist the server no longer has is dropped. Ported
    /// from the page that used to run this in a `use_effect`, where it stopped
    /// the moment the user navigated away.
    pub fn spawn_sync(
        self: &Arc<Self>,
        runner: &crate::jobs::JobRunner,
    ) -> Result<api::JobRef, ApiError> {
        let service = self.clone();
        runner.start(api::JobKind::PlaylistSync, move |ctx| async move {
            service.sync(&ctx).await
        })
    }

    async fn sync(&self, ctx: &crate::jobs::JobCtx) -> Result<(), ApiError> {
        let source = self.active_source();
        if !source.capabilities().sync {
            return Err(ApiError::unsupported(
                "the active source has no playlist sync",
            ));
        }
        let existing = self
            .db
            .load_playlists(&self.config().active_source)
            .await
            .map_err(db_error)?
            .playlists;

        ctx.progress("fetching playlists", None, None, None);
        let metas = source.fetch_playlists().await.map_err(source_error)?;
        let total = metas.len() as u64;

        for meta in &metas {
            // A manually chosen cover is the user's, not the server's, so it
            // survives the refresh.
            let existing_cover = existing
                .iter()
                .find(|playlist| playlist.id == meta.id)
                .and_then(|playlist| playlist.cover_path.clone())
                .map(|path| path.to_string_lossy().into_owned());
            let _ = source
                .upsert_playlist_meta(
                    &meta.id,
                    &meta.name,
                    existing_cover.as_deref(),
                    meta.image_tag.as_deref(),
                )
                .await;
        }
        self.session.invalidate(Table::Playlists);

        let mut seen: std::collections::HashSet<reader::TrackId> = std::collections::HashSet::new();
        for (index, meta) in metas.iter().enumerate() {
            if ctx.cancelled() {
                return Ok(());
            }
            ctx.progress(
                "fetching playlists",
                Some(index as u64 + 1),
                Some(total),
                None,
            );
            let entries = source
                .fetch_playlist_entries(&meta.id)
                .await
                .unwrap_or_default();
            let track_keys: Vec<String> = entries
                .iter()
                .map(|track| track.id.key().to_string())
                .filter(|key| !key.is_empty())
                .collect();
            if source
                .set_playlist_tracks(&meta.id, &track_keys)
                .await
                .is_ok()
            {
                self.session.invalidate(Table::Playlists);
            }
            // Playlists overlap, so a track is only written the first time it
            // is seen across the whole walk.
            let fresh: Vec<reader::Track> = entries
                .into_iter()
                .filter(|track| seen.insert(track.id.clone()))
                .collect();
            for chunk in fresh.chunks(100) {
                let _ = source.upsert_tracks(chunk).await;
            }
            self.session.invalidate(Table::Tracks);
        }

        if ctx.cancelled() {
            return Ok(());
        }
        for stale in existing
            .iter()
            .filter(|playlist| !metas.iter().any(|meta| meta.id == playlist.id))
        {
            let _ = source.delete_playlist(&stale.id).await;
        }
        // The stamp is what stops the automatic sync running twice; only the
        // source that gates on it writes one.
        if source.capabilities().albums == server::source::AlbumType::YtMusic {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_secs())
                .unwrap_or_default();
            let mut stamps: serde_json::Value = self
                .db
                .meta_get("yt_sync", "timestamps")
                .await
                .ok()
                .flatten()
                .and_then(|raw| serde_json::from_str(&raw).ok())
                .unwrap_or_else(|| serde_json::json!({}));
            stamps["last_yt_playlists_sync_at"] = serde_json::json!(now);
            let _ = source
                .set_meta("yt_sync", "timestamps", &stamps.to_string())
                .await;
        }
        self.session.invalidate(Table::Tracks);
        self.session.invalidate(Table::Playlists);
        Ok(())
    }
}
