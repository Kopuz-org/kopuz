use std::path::{Component, Path, PathBuf};

use async_trait::async_trait;
use config::Source;
use db::Db;
use tokio::sync::OnceCell;

use super::{
    AlbumType, ArtistView, AuthOutcome, Capabilities, FavoritesSync, MediaSource, PlaylistOps,
    SourceError, StreamInfo,
};

pub const PORTABLE_LIBRARY_DB_FILENAME: &str = ".kopuz-library.db";
const PORTABLE_REF_PREFIX: &str = "kopuz-root-v1:";
const PORTABLE_INIT_KEY: &str = "initialized-v1";
const PORTABLE_INIT_KIND: &str = "portable-library";
const PORTABLE_ACTIVITY_INIT_KEY: &str = "activity-initialized-v1";

pub(super) struct LocalSource {
    pub(super) db: Db,
    pub(super) source: Source,
    directories: Vec<PathBuf>,
    portable: OnceCell<Option<Db>>,
}

impl LocalSource {
    pub(super) fn new(db: Db, source: Source, directories: Vec<PathBuf>) -> Self {
        Self {
            db,
            source,
            directories,
            portable: OnceCell::new(),
        }
    }

    fn portable_path(&self) -> Option<PathBuf> {
        self.directories
            .first()
            .map(|root| root.join(PORTABLE_LIBRARY_DB_FILENAME))
    }

    async fn portable_db(&self) -> Option<&Db> {
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
                    portable.create_folder(&folder.id, &folder.name).await?;
                    for playlist_id in &folder.playlist_ids {
                        portable
                            .set_playlist_folder(playlist_id, Some(&folder.id))
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

    async fn metadata_store(&self) -> (&Db, Source) {
        match self.portable_db().await {
            Some(portable) => (portable, Source::Local),
            None => (&self.db, self.source.clone()),
        }
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
        let (db, source) = self.metadata_store().await;
        Ok(db
            .favorites(source.as_str())
            .await?
            .iter()
            .map(|reference| self.decode_ref(reference))
            .collect())
    }

    async fn is_favorite(&self, reference: &str) -> bool {
        let encoded = self.encode_ref(reference);
        let (db, source) = self.metadata_store().await;
        db.is_favorite(source.as_str(), &encoded)
            .await
            .unwrap_or(false)
    }

    async fn set_favorite(&self, reference: &str, on: bool) -> Result<(), SourceError> {
        let encoded = self.encode_ref(reference);
        let (db, source) = self.metadata_store().await;
        db.set_favorite(source.as_str(), &encoded, on).await?;
        db.clear_favorite_dirty(source.as_str(), &encoded)
            .await
            .map_err(SourceError::from)
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
        let (db, source) = self.metadata_store().await;
        let mut store = db.load_playlists(&source).await?;
        for playlist in &mut store.playlists {
            for reference in &mut playlist.tracks {
                *reference = self.decode_ref(reference);
            }
            if let Some(cover) = playlist.cover_path.as_mut() {
                *cover = PathBuf::from(self.decode_ref(&cover.to_string_lossy()));
            }
        }
        Ok(store)
    }

    async fn recently_played(&self, limit: u32) -> Result<Vec<String>, SourceError> {
        let (db, source) = self.metadata_store().await;
        Ok(db
            .recently_played(&source, limit)
            .await?
            .iter()
            .map(|reference| self.decode_ref(reference))
            .collect())
    }

    async fn sync_portable_activity(&self) -> Result<Vec<(String, u64)>, SourceError> {
        let Some(portable) = self.portable_db().await else {
            return Ok(Vec::new());
        };
        let decoded_counts: Vec<(String, u64)> = portable
            .listen_counts()
            .await?
            .into_iter()
            .map(|(reference, count)| (self.decode_ref(&reference), count))
            .collect();
        self.db
            .merge_listen_counts(&self.source, &decoded_counts)
            .await?;

        let recents = portable.recently_played(&Source::Local, 50).await?;
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
        let encoded = self.encode_ref(track_uid);
        let Some(portable) = self.portable_db().await else {
            return self
                .db
                .bump_listen_count(&self.source, track_uid)
                .await
                .map_err(SourceError::from);
        };
        portable.bump_listen_count(&Source::Local, &encoded).await?;
        if let Err(error) = self.db.bump_listen_count(&self.source, track_uid).await {
            tracing::warn!(%error, "failed to mirror shared listen count into app database");
        }
        Ok(())
    }

    async fn record_recent(&self, track_key: &str) -> Result<(), SourceError> {
        let encoded = self.encode_ref(track_key);
        let Some(portable) = self.portable_db().await else {
            return self
                .db
                .push_recent(&self.source, track_key)
                .await
                .map_err(SourceError::from);
        };
        portable.push_recent(&Source::Local, &encoded).await?;
        if let Err(error) = self.db.push_recent(&self.source, track_key).await {
            tracing::warn!(%error, "failed to mirror shared recent history into app database");
        }
        Ok(())
    }

    async fn add_to_playlist(
        &self,
        playlist_id: &str,
        item_refs: &[String],
    ) -> Result<Vec<String>, SourceError> {
        let encoded = self.encode_refs(item_refs);
        let (db, source) = self.metadata_store().await;
        db.add_playlist_tracks(&source, playlist_id, &encoded)
            .await?;
        Ok(item_refs.to_vec())
    }

    async fn create_playlist(
        &self,
        name: &str,
        item_refs: &[String],
    ) -> Result<String, SourceError> {
        let id = uuid::Uuid::new_v4().to_string();
        let encoded = self.encode_refs(item_refs);
        let (db, source) = self.metadata_store().await;
        db.upsert_playlist_meta(&source, &id, name, None, None)
            .await?;
        db.set_playlist_tracks(&source, &id, &encoded).await?;
        Ok(id)
    }

    async fn remove_from_playlist(
        &self,
        playlist_id: &str,
        track: &reader::Track,
        _position: usize,
    ) -> Result<(), SourceError> {
        let reference = self.encode_ref(track.id.key().as_ref());
        let (db, source) = self.metadata_store().await;
        db.remove_playlist_tracks(&source, playlist_id, &[reference])
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
        self.set_playlist_tracks(playlist_id, ordered_refs).await
    }

    async fn set_playlist_tracks(
        &self,
        playlist_id: &str,
        refs: &[String],
    ) -> Result<(), SourceError> {
        let encoded = self.encode_refs(refs);
        let (db, source) = self.metadata_store().await;
        db.set_playlist_tracks(&source, playlist_id, &encoded)
            .await
            .map_err(SourceError::from)
    }

    async fn remove_playlist_tracks(
        &self,
        playlist_id: &str,
        refs: &[String],
    ) -> Result<(), SourceError> {
        let encoded = self.encode_refs(refs);
        let (db, source) = self.metadata_store().await;
        db.remove_playlist_tracks(&source, playlist_id, &encoded)
            .await
            .map_err(SourceError::from)
    }

    async fn delete_playlist(&self, playlist_id: &str) -> Result<(), SourceError> {
        let (db, source) = self.metadata_store().await;
        db.delete_playlist(&source, playlist_id)
            .await
            .map_err(SourceError::from)
    }

    async fn set_playlist_cover(
        &self,
        playlist_id: &str,
        name: &str,
        image_path: &Path,
        image_tag: Option<&str>,
    ) -> Result<(), SourceError> {
        let cover = self.encode_ref(&image_path.to_string_lossy());
        self.upsert_playlist_meta(playlist_id, name, Some(&cover), image_tag)
            .await
    }

    async fn create_folder(&self, id: &str, name: &str) -> Result<(), SourceError> {
        let (db, _) = self.metadata_store().await;
        db.create_folder(id, name).await.map_err(SourceError::from)
    }

    async fn rename_folder(&self, id: &str, name: &str) -> Result<(), SourceError> {
        let (db, _) = self.metadata_store().await;
        db.rename_folder(id, name).await.map_err(SourceError::from)
    }

    async fn delete_folder(&self, id: &str) -> Result<(), SourceError> {
        let (db, _) = self.metadata_store().await;
        db.delete_folder(id).await.map_err(SourceError::from)
    }

    async fn set_playlist_folder(
        &self,
        playlist_ref: &str,
        folder_id: Option<&str>,
    ) -> Result<(), SourceError> {
        let (db, _) = self.metadata_store().await;
        db.set_playlist_folder(playlist_ref, folder_id)
            .await
            .map_err(SourceError::from)
    }

    async fn upsert_playlist_meta(
        &self,
        playlist_id: &str,
        name: &str,
        cover_path: Option<&str>,
        image_tag: Option<&str>,
    ) -> Result<(), SourceError> {
        let cover = cover_path.map(|path| self.encode_ref(path));
        let (db, source) = self.metadata_store().await;
        db.upsert_playlist_meta(&source, playlist_id, name, cover.as_deref(), image_tag)
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
