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

use crate::error::{db_error, source_error};

use std::sync::Arc;

use api::{ApiError, PlaylistCatalog, PlaylistFolderInfo, PlaylistInfo, PlaylistReorder, Table};

use crate::session::SessionHandle;

pub struct PlaylistService {
    db: db::Db,
    session: SessionHandle,
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
        let entry = self
            .entries(playlist_id)
            .await?
            .into_iter()
            .nth(index)
            .ok_or_else(|| ApiError::not_found("no such playlist entry"))?;
        let mut track = self
            .db
            .tracks_by_keys(&source, std::slice::from_ref(&entry.key))
            .await
            .map_err(db_error)?
            .into_iter()
            .next()
            .ok_or_else(|| ApiError::not_found("the playlist entry names an unknown track"))?;
        track.playlist_item_id = entry.item_id;
        Ok(track)
    }

    async fn entries(&self, playlist_id: &str) -> Result<Vec<reader::PlaylistEntry>, ApiError> {
        self.db
            .playlist_entries(&self.config().active_source, playlist_id)
            .await
            .map_err(db_error)
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
    ///
    /// Gated on `sync`, not on the playlist ops. A local playlist has no
    /// remote to pull from: the default entry fetch answers with an empty
    /// page, and the closing sweep then takes the rows that *are* its
    /// contents. Local advertises `Reorder`, so gating on the ops let it in.
    pub async fn refresh(&self, id: &str) -> Result<(), ApiError> {
        let source = self.active_source();
        if !source.capabilities().sync {
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
            .map_err(db_error)?
            .and_then(|raw| raw.parse().ok())
            .unwrap_or(0);
        if last <= now && now - last < 15 * 60 {
            return Ok(());
        }

        let epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as i64)
            .unwrap_or_default();
        let mut cursor: Option<String> = None;
        let mut position: i64 = 0;
        let mut cursors = std::collections::HashSet::new();

        loop {
            let page = source
                .fetch_playlist_entries_page(id, cursor.clone())
                .await
                .map_err(source_error)?;
            let next = page.next.clone();
            let page_refs: Vec<reader::PlaylistEntry> = page
                .tracks
                .iter()
                .map(reader::PlaylistEntry::from_track)
                .filter(|entry| !entry.key.is_empty())
                .collect();
            for chunk in page.tracks.chunks(100) {
                source.upsert_tracks(chunk).await.map_err(source_error)?;
            }
            source
                .upsert_playlist_tracks_page(id, &page_refs, position, epoch)
                .await
                .map_err(source_error)?;
            position += page_refs.len() as i64;
            self.session.invalidate(Table::Tracks);
            self.session.invalidate(Table::Playlists);
            match next {
                Some(next) => {
                    if !cursors.insert(next.clone()) {
                        return Err(ApiError::internal("playlist pagination repeated a cursor"));
                    }
                    cursor = Some(next);
                }
                None => break,
            }
        }

        source
            .sweep_playlist_tracks(id, epoch)
            .await
            .map_err(source_error)?;
        source
            .set_meta("pl_pull", id, &now.to_string())
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

impl PlaylistService {
    /// Pull the server's playlists and their contents.
    ///
    /// Publish the listing before fetching entries; drop absent playlists
    /// after the full sync succeeds.
    pub fn spawn_sync(
        self: &Arc<Self>,
        runner: &crate::jobs::JobRunner,
    ) -> Result<api::JobRef, ApiError> {
        let service = self.clone();
        runner.start(api::JobKind::PlaylistSync, move |ctx| async move {
            let source = service.active_source();
            let result = service.sync(&ctx, &source).await;
            if result.is_ok() && !ctx.cancelled() {
                crate::auto_sync::mark_synced(
                    &service.db,
                    api::JobKind::PlaylistSync,
                    source.source(),
                )
                .await;
            }
            result
        })
    }

    async fn sync(
        &self,
        ctx: &crate::jobs::JobCtx,
        source: &server::source::ActiveSource,
    ) -> Result<(), ApiError> {
        if !source.capabilities().sync {
            return Err(ApiError::unsupported(
                "the active source has no playlist sync",
            ));
        }
        let existing = self
            .db
            .load_playlists(source.source())
            .await
            .map_err(db_error)?
            .playlists;

        ctx.progress("fetching playlists", None, None, None);
        let metas = source.fetch_playlists().await.map_err(source_error)?;
        let total = metas.len() as u64;

        for meta in &metas {
            let existing_cover = existing
                .iter()
                .find(|playlist| playlist.id == meta.id)
                .and_then(|playlist| playlist.cover_path.clone())
                .map(|path| path.to_string_lossy().into_owned());
            source
                .upsert_playlist_meta(
                    &meta.id,
                    &meta.name,
                    existing_cover.as_deref(),
                    meta.image_tag.as_deref(),
                )
                .await
                .map_err(source_error)?;
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
            sync_entries(source.as_ref(), &meta.id, &mut seen).await?;
            self.session.invalidate(Table::Playlists);
            self.session.invalidate(Table::Tracks);
        }

        if ctx.cancelled() {
            return Ok(());
        }
        for stale in existing
            .iter()
            .filter(|playlist| !metas.iter().any(|meta| meta.id == playlist.id))
        {
            source
                .delete_playlist(&stale.id)
                .await
                .map_err(source_error)?;
        }
        self.session.invalidate(Table::Tracks);
        self.session.invalidate(Table::Playlists);
        Ok(())
    }
}

async fn sync_entries(
    source: &dyn server::source::MediaSource,
    id: &str,
    seen: &mut std::collections::HashSet<reader::TrackId>,
) -> Result<(), ApiError> {
    let entries = source
        .fetch_playlist_entries(id)
        .await
        .map_err(source_error)?;
    let track_keys: Vec<reader::PlaylistEntry> = entries
        .iter()
        .map(reader::PlaylistEntry::from_track)
        .filter(|entry| !entry.key.is_empty())
        .collect();

    let fresh: Vec<reader::Track> = entries
        .into_iter()
        .filter(|track| seen.insert(track.id.clone()))
        .collect();
    for chunk in fresh.chunks(100) {
        source.upsert_tracks(chunk).await.map_err(source_error)?;
    }
    source
        .set_playlist_tracks(id, &track_keys)
        .await
        .map_err(source_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::sync_entries;
    use server::source::{AuthOutcome, Capabilities, MediaSource, SourceError, StreamInfo};

    #[derive(Clone, Copy)]
    enum Reply {
        FetchFailure,
        WriteFailure,
        Empty,
    }

    struct Source {
        inner: Box<dyn MediaSource>,
        reply: Reply,
    }

    #[async_trait::async_trait]
    impl MediaSource for Source {
        fn source(&self) -> &config::Source {
            self.inner.source()
        }
        fn db(&self) -> &db::Db {
            self.inner.db()
        }
        fn capabilities(&self) -> Capabilities {
            self.inner.capabilities()
        }
        async fn add_to_playlist(
            &self,
            id: &str,
            keys: &[String],
        ) -> Result<Vec<String>, SourceError> {
            self.inner.add_to_playlist(id, keys).await
        }
        async fn create_playlist(
            &self,
            name: &str,
            keys: &[String],
        ) -> Result<String, SourceError> {
            self.inner.create_playlist(name, keys).await
        }
        async fn remove_from_playlist(
            &self,
            id: &str,
            track: &reader::Track,
            position: usize,
        ) -> Result<(), SourceError> {
            self.inner.remove_from_playlist(id, track, position).await
        }
        async fn resolve_stream(&self, key: &str) -> Result<StreamInfo, SourceError> {
            self.inner.resolve_stream(key).await
        }
        async fn validate(&self) -> AuthOutcome {
            self.inner.validate().await
        }
        async fn fetch_favorites(&self) -> Result<Vec<String>, SourceError> {
            self.inner.fetch_favorites().await
        }
        async fn push_favorite(&self, key: &str, on: bool) -> Result<(), SourceError> {
            self.inner.push_favorite(key, on).await
        }
        async fn fetch_playlist_entries(&self, _: &str) -> Result<Vec<reader::Track>, SourceError> {
            match self.reply {
                Reply::FetchFailure => Err(SourceError::Connectivity),
                Reply::Empty => Ok(Vec::new()),
                Reply::WriteFailure => Ok(vec![
                    serde_json::from_value(serde_json::json!({
                        "id": {"Local": "/new.flac"}, "album_id": "album", "title": "New",
                        "artist": "Artist", "album": "Album", "duration": 60, "khz": 44,
                        "track_number": null, "disc_number": null
                    }))
                    .expect("track"),
                ]),
            }
        }
        async fn upsert_tracks(&self, tracks: &[reader::Track]) -> Result<(), SourceError> {
            match self.reply {
                Reply::WriteFailure => Err(SourceError::Backend("write failed".into())),
                _ => self.inner.upsert_tracks(tracks).await,
            }
        }
    }

    #[tokio::test]
    async fn failed_fetch_or_write_preserves_membership_but_successful_empty_clears_it() {
        for reply in [Reply::FetchFailure, Reply::WriteFailure, Reply::Empty] {
            let dir = tempfile::tempdir().expect("tempdir");
            let database = db::init(&dir.path().join("playlist.db")).await.expect("db");
            let source = Source {
                inner: server::source::local(database.clone(), config::Source::default()),
                reply,
            };
            let id = source
                .create_playlist("Saved", &["/old.flac".into()])
                .await
                .expect("playlist");
            let result = sync_entries(&source, &id, &mut Default::default()).await;
            let entries = database
                .playlist_entries(source.source(), &id)
                .await
                .expect("entries");
            match reply {
                Reply::Empty => {
                    assert!(result.is_ok());
                    assert!(entries.is_empty());
                }
                _ => {
                    assert!(result.is_err());
                    assert_eq!(entries.len(), 1);
                    assert_eq!(entries[0].key, "/old.flac");
                }
            }
        }
    }
}
