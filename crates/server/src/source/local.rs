use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use async_trait::async_trait;
use config::Source;
use db::Db;
use tokio::sync::{Mutex, Notify, OnceCell};

use super::{
    AlbumType, ArtistView, AuthOutcome, Capabilities, FavoritesSync, MediaSource, PlaylistOps,
    SourceError, StreamInfo,
};

pub const PORTABLE_LIBRARY_DB_FILENAME: &str = ".kopuz-library.db";
const PORTABLE_REF_PREFIX: &str = "kopuz-root-v1:";
const PORTABLE_INIT_KEY: &str = "initialized-v1";
const PORTABLE_INIT_KIND: &str = "portable-library";
const PORTABLE_ACTIVITY_INIT_KEY: &str = "activity-initialized-v1";

enum PortableMutation {
    FlushFavorites,
    AddPlaylistTracks {
        playlist_id: String,
        refs: Vec<String>,
    },
    CreatePlaylist {
        id: String,
        name: String,
        refs: Vec<String>,
    },
    RemovePlaylistTracks {
        playlist_id: String,
        refs: Vec<String>,
    },
    SetPlaylistTracks {
        playlist_id: String,
        refs: Vec<String>,
    },
    DeletePlaylist {
        playlist_id: String,
    },
    UpsertPlaylistMeta {
        playlist_id: String,
        name: String,
        cover_path: Option<String>,
        image_tag: Option<String>,
    },
    CreateFolder {
        id: String,
        name: String,
    },
    RenameFolder {
        id: String,
        name: String,
    },
    DeleteFolder {
        id: String,
    },
    SetPlaylistFolder {
        playlist_ref: String,
        folder_id: Option<String>,
    },
    BumpListenCount {
        track_uid: String,
    },
    RecordRecent {
        track_key: String,
    },
}

#[derive(Clone)]
pub(super) struct LocalSource {
    pub(super) db: Db,
    pub(super) source: Source,
    directories: Vec<PathBuf>,
    portable: Arc<OnceCell<Option<Db>>>,
    portable_failed: Arc<AtomicBool>,
    portable_pending: Arc<AtomicUsize>,
    portable_idle: Arc<Notify>,
    portable_write_lock: Arc<Mutex<()>>,
}

impl LocalSource {
    pub(super) fn new(db: Db, source: Source, directories: Vec<PathBuf>) -> Self {
        Self {
            db,
            source,
            directories,
            portable: Arc::new(OnceCell::new()),
            portable_failed: Arc::new(AtomicBool::new(false)),
            portable_pending: Arc::new(AtomicUsize::new(0)),
            portable_idle: Arc::new(Notify::new()),
            portable_write_lock: Arc::new(Mutex::new(())),
        }
    }

    fn portable_path(&self) -> Option<PathBuf> {
        self.directories
            .first()
            .map(|root| root.join(PORTABLE_LIBRARY_DB_FILENAME))
    }

    async fn portable_db(&self) -> Option<&Db> {
        if self.portable_failed.load(Ordering::Acquire) {
            return None;
        }
        self.portable
            .get_or_init(|| async {
                let path = self.portable_path()?;
                let portable = match db::init_portable(&path).await {
                    Ok(db) => db,
                    Err(error) => {
                        tracing::warn!(
                            %error,
                            path = %path.display(),
                            "portable library metadata is unavailable; using the app database"
                        );
                        return None;
                    }
                };
                if let Err(error) = self.seed_portable(&portable).await {
                    tracing::warn!(
                        %error,
                        path = %path.display(),
                        "failed to initialize portable library metadata; using the app database"
                    );
                    return None;
                }
                Some(portable)
            })
            .await
            .as_ref()
    }

    fn disable_portable(&self, error: &SourceError) {
        if !self.portable_failed.swap(true, Ordering::AcqRel) {
            tracing::warn!(
                %error,
                "shared library database failed; using the app database for this session"
            );
        }
    }

    async fn sync_portable_favorites(&self, portable: &Db) -> Result<(), SourceError> {
        let dirty_likes = self.db.dirty_favorites(self.source.as_str()).await?;
        for reference in dirty_likes {
            let encoded = self.encode_ref(&reference);
            portable
                .set_favorite(Source::Local.as_str(), &encoded, true)
                .await?;
            portable
                .clear_favorite_dirty(Source::Local.as_str(), &encoded)
                .await?;
            self.db
                .clear_favorite_dirty(self.source.as_str(), &reference)
                .await?;
        }

        let dirty_unlikes = self.db.dirty_unlikes(self.source.as_str()).await?;
        for reference in dirty_unlikes {
            let encoded = self.encode_ref(&reference);
            portable
                .set_favorite(Source::Local.as_str(), &encoded, false)
                .await?;
            portable
                .clear_favorite_dirty(Source::Local.as_str(), &encoded)
                .await?;
            self.db
                .clear_favorite_dirty(self.source.as_str(), &reference)
                .await?;
        }

        let favorites: Vec<String> = portable
            .favorites(Source::Local.as_str())
            .await?
            .iter()
            .map(|reference| self.decode_ref(reference))
            .collect();
        self.db
            .replace_favorites_clean(self.source.as_str(), &favorites)
            .await
            .map_err(SourceError::from)
    }

    async fn sync_portable_playlists(&self, portable: &Db) -> Result<(), SourceError> {
        let mut store = portable.load_playlists(&Source::Local).await?;
        for playlist in &mut store.playlists {
            for reference in &mut playlist.tracks {
                *reference = self.decode_ref(reference);
            }
            if let Some(cover) = playlist.cover_path.as_mut() {
                *cover = PathBuf::from(self.decode_ref(&cover.to_string_lossy()));
            }
        }
        self.db
            .replace_playlist_store(&self.source, &store)
            .await
            .map_err(SourceError::from)
    }

    fn queue_portable_mutation(&self, mutation: PortableMutation) {
        if self.portable_path().is_none() || self.portable_failed.load(Ordering::Acquire) {
            return;
        }
        self.portable_pending.fetch_add(1, Ordering::AcqRel);
        let source = self.clone();
        tokio::spawn(async move {
            let _write_guard = source.portable_write_lock.lock().await;
            if let Some(portable) = source.portable_db().await
                && let Err(error) = source.apply_portable_mutation(portable, mutation).await
            {
                source.disable_portable(&error);
            }
            if source.portable_pending.fetch_sub(1, Ordering::AcqRel) == 1 {
                source.portable_idle.notify_waiters();
            }
        });
    }

    async fn wait_for_portable_mutations(&self) {
        while self.portable_pending.load(Ordering::Acquire) != 0 {
            let idle = self.portable_idle.notified();
            if self.portable_pending.load(Ordering::Acquire) == 0 {
                break;
            }
            idle.await;
        }
    }

    async fn apply_portable_mutation(
        &self,
        portable: &Db,
        mutation: PortableMutation,
    ) -> Result<(), SourceError> {
        match mutation {
            PortableMutation::FlushFavorites => self.sync_portable_favorites(portable).await,
            PortableMutation::AddPlaylistTracks { playlist_id, refs } => portable
                .add_playlist_tracks(&Source::Local, &playlist_id, &self.encode_refs(&refs))
                .await
                .map_err(SourceError::from),
            PortableMutation::CreatePlaylist { id, name, refs } => {
                portable
                    .upsert_playlist_meta(&Source::Local, &id, &name, None, None)
                    .await?;
                portable
                    .set_playlist_tracks(&Source::Local, &id, &self.encode_refs(&refs))
                    .await
                    .map_err(SourceError::from)
            }
            PortableMutation::RemovePlaylistTracks { playlist_id, refs } => portable
                .remove_playlist_tracks(&Source::Local, &playlist_id, &self.encode_refs(&refs))
                .await
                .map_err(SourceError::from),
            PortableMutation::SetPlaylistTracks { playlist_id, refs } => portable
                .set_playlist_tracks(&Source::Local, &playlist_id, &self.encode_refs(&refs))
                .await
                .map_err(SourceError::from),
            PortableMutation::DeletePlaylist { playlist_id } => portable
                .delete_playlist(&Source::Local, &playlist_id)
                .await
                .map_err(SourceError::from),
            PortableMutation::UpsertPlaylistMeta {
                playlist_id,
                name,
                cover_path,
                image_tag,
            } => {
                let cover = cover_path.map(|path| self.encode_ref(&path));
                portable
                    .upsert_playlist_meta(
                        &Source::Local,
                        &playlist_id,
                        &name,
                        cover.as_deref(),
                        image_tag.as_deref(),
                    )
                    .await
                    .map_err(SourceError::from)
            }
            PortableMutation::CreateFolder { id, name } => portable
                .create_folder(&Source::Local, &id, &name)
                .await
                .map_err(SourceError::from),
            PortableMutation::RenameFolder { id, name } => portable
                .rename_folder(&Source::Local, &id, &name)
                .await
                .map_err(SourceError::from),
            PortableMutation::DeleteFolder { id } => portable
                .delete_folder(&Source::Local, &id)
                .await
                .map_err(SourceError::from),
            PortableMutation::SetPlaylistFolder {
                playlist_ref,
                folder_id,
            } => portable
                .set_playlist_folder(&Source::Local, &playlist_ref, folder_id.as_deref())
                .await
                .map_err(SourceError::from),
            PortableMutation::BumpListenCount { track_uid } => portable
                .bump_listen_count(&Source::Local, &self.encode_ref(&track_uid))
                .await
                .map_err(SourceError::from),
            PortableMutation::RecordRecent { track_key } => portable
                .push_recent(&Source::Local, &self.encode_ref(&track_key))
                .await
                .map_err(SourceError::from),
        }
    }

    /// Import metadata from the app DB once when a library gets its portable
    /// DB. The marker prevents a deliberately emptied portable library from
    /// resurrecting stale data on the next launch.
    async fn seed_portable(&self, portable: &Db) -> Result<(), SourceError> {
        let metadata_initialized = portable
            .meta_get(PORTABLE_INIT_KEY, PORTABLE_INIT_KIND)
            .await?
            .is_some();
        if !metadata_initialized {
            let existing_favorites = portable.favorites(Source::Local.as_str()).await?;
            let existing_playlists = portable.load_playlists(&Source::Local).await?;
            if existing_favorites.is_empty()
                && existing_playlists.playlists.is_empty()
                && existing_playlists.folders.is_empty()
            {
                let favorites = self.db.favorites(self.source.as_str()).await?;
                let favorites: Vec<String> = favorites
                    .iter()
                    .map(|reference| self.encode_ref(reference))
                    .collect();
                portable
                    .replace_favorites_clean(Source::Local.as_str(), &favorites)
                    .await?;

                let store = self.db.load_playlists(&self.source).await?;
                for playlist in &store.playlists {
                    let cover = playlist
                        .cover_path
                        .as_ref()
                        .map(|path| self.encode_ref(&path.to_string_lossy()));
                    portable
                        .upsert_playlist_meta(
                            &Source::Local,
                            &playlist.id,
                            &playlist.name,
                            cover.as_deref(),
                            playlist.image_tag.as_deref(),
                        )
                        .await?;
                    let tracks: Vec<String> = playlist
                        .tracks
                        .iter()
                        .map(|reference| self.encode_ref(reference))
                        .collect();
                    portable
                        .set_playlist_tracks(&Source::Local, &playlist.id, &tracks)
                        .await?;
                }
                for folder in &store.folders {
                    portable
                        .create_folder(&Source::Local, &folder.id, &folder.name)
                        .await?;
                    for playlist_id in &folder.playlist_ids {
                        portable
                            .set_playlist_folder(&Source::Local, playlist_id, Some(&folder.id))
                            .await?;
                    }
                }
            }

            portable
                .meta_put(PORTABLE_INIT_KEY, PORTABLE_INIT_KIND, "1")
                .await?;
        }

        let activity_initialized = portable
            .meta_get(PORTABLE_ACTIVITY_INIT_KEY, PORTABLE_INIT_KIND)
            .await?
            .is_some();
        if !activity_initialized {
            let existing_counts = portable.listen_counts().await?;
            let existing_recents = portable.recently_played(&Source::Local, 1).await?;
            if existing_counts.is_empty() {
                let counts = self.db.listen_counts().await?;
                let portable_counts: Vec<(String, u64)> = counts
                    .into_iter()
                    .filter_map(|(key, count)| {
                        let reference = self.local_count_ref(&key)?;
                        let encoded = self.encode_ref(reference);
                        encoded
                            .starts_with(PORTABLE_REF_PREFIX)
                            .then_some((encoded, count))
                    })
                    .collect();
                portable
                    .merge_listen_counts(&Source::Local, &portable_counts)
                    .await?;
            }

            if existing_recents.is_empty() {
                let recents = self.db.recently_played(&self.source, 50).await?;
                for reference in recents.iter().rev() {
                    let encoded = self.encode_ref(reference);
                    if encoded.starts_with(PORTABLE_REF_PREFIX) {
                        portable.push_recent(&Source::Local, &encoded).await?;
                    }
                }
            }
            portable
                .meta_put(PORTABLE_ACTIVITY_INIT_KEY, PORTABLE_INIT_KIND, "1")
                .await?;
        }

        Ok(())
    }

    fn encode_ref(&self, reference: &str) -> String {
        let path = Path::new(reference);
        for (root_index, root) in self.directories.iter().enumerate() {
            let Ok(relative) = path.strip_prefix(root) else {
                continue;
            };
            let mut segments = Vec::new();
            for component in relative.components() {
                match component {
                    Component::Normal(segment) => {
                        segments.push(segment.to_string_lossy().into_owned());
                    }
                    Component::CurDir => {}
                    Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                        return reference.to_owned();
                    }
                }
            }
            return format!("{PORTABLE_REF_PREFIX}{root_index}:{}", segments.join("/"));
        }
        reference.to_owned()
    }

    fn decode_ref(&self, reference: &str) -> String {
        let Some(encoded) = reference.strip_prefix(PORTABLE_REF_PREFIX) else {
            return reference.to_owned();
        };
        let Some((root_index, relative)) = encoded.split_once(':') else {
            return reference.to_owned();
        };
        let Some(root) = root_index
            .parse::<usize>()
            .ok()
            .and_then(|index| self.directories.get(index))
        else {
            return reference.to_owned();
        };
        let mut path = root.clone();
        if !relative.is_empty() {
            for segment in relative.split('/') {
                if segment.is_empty() || segment == "." || segment == ".." {
                    return reference.to_owned();
                }
                path.push(segment);
            }
        }
        path.to_string_lossy().into_owned()
    }

    fn encode_refs(&self, references: &[String]) -> Vec<String> {
        references
            .iter()
            .map(|reference| self.encode_ref(reference))
            .collect()
    }

    fn local_count_ref<'a>(&self, key: &'a str) -> Option<&'a str> {
        match &self.source {
            Source::Local => Some(key),
            Source::LocalLibrary(id) => key.strip_prefix(id)?.strip_prefix('|'),
            Source::Server(_) => None,
        }
    }
}

#[async_trait]
impl MediaSource for LocalSource {
    fn source(&self) -> &Source {
        &self.source
    }

    fn db(&self) -> &Db {
        &self.db
    }

    fn portable_metadata_path(&self) -> Option<PathBuf> {
        self.portable_path()
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

    async fn favorites(&self) -> Result<Vec<String>, SourceError> {
        self.db
            .favorites(self.source.as_str())
            .await
            .map_err(SourceError::from)
    }

    async fn is_favorite(&self, reference: &str) -> bool {
        self.db
            .is_favorite(self.source.as_str(), reference)
            .await
            .unwrap_or(false)
    }

    async fn set_favorite(&self, reference: &str, on: bool) -> Result<(), SourceError> {
        self.db
            .set_favorite(self.source.as_str(), reference, on)
            .await?;
        if self.portable_path().is_none() {
            return self
                .db
                .clear_favorite_dirty(self.source.as_str(), reference)
                .await
                .map_err(SourceError::from);
        }

        self.queue_portable_mutation(PortableMutation::FlushFavorites);
        Ok(())
    }

    async fn record_favorite(&self, track: &reader::Track, on: bool) -> Result<(), SourceError> {
        let key = track.id.key();
        if key.trim().is_empty() {
            return Ok(());
        }
        if on {
            let _ = self
                .db
                .upsert_tracks(&self.source, std::slice::from_ref(track))
                .await;
        }
        self.set_favorite(key.as_ref(), on).await
    }

    async fn load_playlists(&self) -> Result<reader::PlaylistStore, SourceError> {
        self.db
            .load_playlists(&self.source)
            .await
            .map_err(SourceError::from)
    }

    async fn recently_played(&self, limit: u32) -> Result<Vec<String>, SourceError> {
        self.db
            .recently_played(&self.source, limit)
            .await
            .map_err(SourceError::from)
    }

    async fn sync_portable_activity(&self) -> Result<Vec<(String, u64)>, SourceError> {
        self.wait_for_portable_mutations().await;
        let _write_guard = self.portable_write_lock.lock().await;
        if self.portable_pending.load(Ordering::Acquire) != 0 {
            return Ok(Vec::new());
        }
        let Some(portable) = self.portable_db().await else {
            return Ok(Vec::new());
        };
        if let Err(error) = self.sync_portable_favorites(portable).await {
            self.disable_portable(&error);
            return Err(error);
        }
        if let Err(error) = self.sync_portable_playlists(portable).await {
            self.disable_portable(&error);
            return Err(error);
        }
        let decoded_counts: Vec<(String, u64)> = match portable.listen_counts().await {
            Ok(counts) => counts
                .into_iter()
                .map(|(reference, count)| (self.decode_ref(&reference), count))
                .collect(),
            Err(error) => {
                let error = SourceError::from(error);
                self.disable_portable(&error);
                return Err(error);
            }
        };
        self.db
            .merge_listen_counts(&self.source, &decoded_counts)
            .await?;

        let recents = match portable.recently_played(&Source::Local, 50).await {
            Ok(recents) => recents,
            Err(error) => {
                let error = SourceError::from(error);
                self.disable_portable(&error);
                return Err(error);
            }
        };
        for reference in recents.iter().rev() {
            self.db
                .push_recent(&self.source, &self.decode_ref(reference))
                .await?;
        }

        Ok(decoded_counts
            .into_iter()
            .map(|(reference, count)| (self.source.listen_count_key(&reference), count))
            .collect())
    }

    async fn bump_listen_count(&self, track_uid: &str) -> Result<(), SourceError> {
        self.db.bump_listen_count(&self.source, track_uid).await?;
        self.queue_portable_mutation(PortableMutation::BumpListenCount {
            track_uid: track_uid.to_owned(),
        });
        Ok(())
    }

    async fn record_recent(&self, track_key: &str) -> Result<(), SourceError> {
        self.db.push_recent(&self.source, track_key).await?;
        self.queue_portable_mutation(PortableMutation::RecordRecent {
            track_key: track_key.to_owned(),
        });
        Ok(())
    }

    async fn add_to_playlist(
        &self,
        playlist_id: &str,
        item_refs: &[String],
    ) -> Result<Vec<String>, SourceError> {
        self.db
            .add_playlist_tracks(&self.source, playlist_id, item_refs)
            .await?;
        self.queue_portable_mutation(PortableMutation::AddPlaylistTracks {
            playlist_id: playlist_id.to_owned(),
            refs: item_refs.to_vec(),
        });
        Ok(item_refs.to_vec())
    }

    async fn create_playlist(
        &self,
        name: &str,
        item_refs: &[String],
    ) -> Result<String, SourceError> {
        let id = uuid::Uuid::new_v4().to_string();
        self.db
            .upsert_playlist_meta(&self.source, &id, name, None, None)
            .await?;
        self.db
            .set_playlist_tracks(&self.source, &id, item_refs)
            .await?;
        self.queue_portable_mutation(PortableMutation::CreatePlaylist {
            id: id.clone(),
            name: name.to_owned(),
            refs: item_refs.to_vec(),
        });
        Ok(id)
    }

    async fn remove_from_playlist(
        &self,
        playlist_id: &str,
        track: &reader::Track,
        _position: usize,
    ) -> Result<(), SourceError> {
        let reference = track.id.key().into_owned();
        self.remove_playlist_tracks(playlist_id, &[reference]).await
    }

    async fn reorder_playlist(
        &self,
        playlist_id: &str,
        ordered_refs: &[String],
        _moved: &reader::Track,
        _new_index: usize,
    ) -> Result<(), SourceError> {
        self.set_playlist_tracks(playlist_id, ordered_refs).await
    }

    async fn set_playlist_tracks(
        &self,
        playlist_id: &str,
        refs: &[String],
    ) -> Result<(), SourceError> {
        self.db
            .set_playlist_tracks(&self.source, playlist_id, refs)
            .await?;
        self.queue_portable_mutation(PortableMutation::SetPlaylistTracks {
            playlist_id: playlist_id.to_owned(),
            refs: refs.to_vec(),
        });
        Ok(())
    }

    async fn remove_playlist_tracks(
        &self,
        playlist_id: &str,
        refs: &[String],
    ) -> Result<(), SourceError> {
        self.db
            .remove_playlist_tracks(&self.source, playlist_id, refs)
            .await?;
        self.queue_portable_mutation(PortableMutation::RemovePlaylistTracks {
            playlist_id: playlist_id.to_owned(),
            refs: refs.to_vec(),
        });
        Ok(())
    }

    async fn delete_playlist(&self, playlist_id: &str) -> Result<(), SourceError> {
        self.db.delete_playlist(&self.source, playlist_id).await?;
        self.queue_portable_mutation(PortableMutation::DeletePlaylist {
            playlist_id: playlist_id.to_owned(),
        });
        Ok(())
    }

    async fn set_playlist_cover(
        &self,
        playlist_id: &str,
        name: &str,
        image_path: &Path,
        image_tag: Option<&str>,
    ) -> Result<(), SourceError> {
        self.upsert_playlist_meta(
            playlist_id,
            name,
            Some(&image_path.to_string_lossy()),
            image_tag,
        )
        .await
    }

    async fn create_folder(&self, id: &str, name: &str) -> Result<(), SourceError> {
        self.db.create_folder(&self.source, id, name).await?;
        self.queue_portable_mutation(PortableMutation::CreateFolder {
            id: id.to_owned(),
            name: name.to_owned(),
        });
        Ok(())
    }

    async fn rename_folder(&self, id: &str, name: &str) -> Result<(), SourceError> {
        self.db.rename_folder(&self.source, id, name).await?;
        self.queue_portable_mutation(PortableMutation::RenameFolder {
            id: id.to_owned(),
            name: name.to_owned(),
        });
        Ok(())
    }

    async fn delete_folder(&self, id: &str) -> Result<(), SourceError> {
        self.db.delete_folder(&self.source, id).await?;
        self.queue_portable_mutation(PortableMutation::DeleteFolder { id: id.to_owned() });
        Ok(())
    }

    async fn set_playlist_folder(
        &self,
        playlist_ref: &str,
        folder_id: Option<&str>,
    ) -> Result<(), SourceError> {
        self.db
            .set_playlist_folder(&self.source, playlist_ref, folder_id)
            .await?;
        self.queue_portable_mutation(PortableMutation::SetPlaylistFolder {
            playlist_ref: playlist_ref.to_owned(),
            folder_id: folder_id.map(str::to_owned),
        });
        Ok(())
    }

    async fn upsert_playlist_meta(
        &self,
        playlist_id: &str,
        name: &str,
        cover_path: Option<&str>,
        image_tag: Option<&str>,
    ) -> Result<(), SourceError> {
        self.db
            .upsert_playlist_meta(&self.source, playlist_id, name, cover_path, image_tag)
            .await?;
        self.queue_portable_mutation(PortableMutation::UpsertPlaylistMeta {
            playlist_id: playlist_id.to_owned(),
            name: name.to_owned(),
            cover_path: cover_path.map(str::to_owned),
            image_tag: image_tag.map(str::to_owned),
        });
        Ok(())
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
