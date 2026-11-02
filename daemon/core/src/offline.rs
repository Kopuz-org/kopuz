//! What plays without the internet: an in-memory mirror of the `offline_tracks` downloads, the offline playback cache beside it, and whether offline mode is on.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use api::ApiError;
use server::playback_ref::PlaybackItemRef;
use tokio::sync::{Mutex, Notify};

use crate::offline_cache::OfflineCache;

#[derive(Clone, Default)]
pub struct OfflineIndex {
    paths: Arc<RwLock<HashMap<String, String>>>,
    /// Held across a row write and the map update that follows it, so two writers cannot leave them disagreeing.
    writer: Arc<Mutex<()>>,
    /// Where a copy found missing is unregistered; an index built without one only forgets it in memory.
    db: Option<db::Db>,
    pruned: Arc<Notify>,
    /// A separate store: its songs are evicted and counted against its cap, downloads never.
    cache: OfflineCache,
    /// Listings and playback keep to what plays without the internet; the source service sets it.
    offline_mode: Arc<AtomicBool>,
}

impl OfflineIndex {
    /// The registered copies, less any whose file is gone, which are unregistered here.
    pub async fn load(db: &db::Db) -> Result<Self, db::DbError> {
        let mut paths = db.offline_tracks().await?;
        let missing: Vec<String> = paths
            .iter()
            .filter(|(_, path)| !Path::new(path).is_file())
            .map(|(id, _)| id.clone())
            .collect();
        for id in &missing {
            db.set_offline_track(id, None).await?;
            paths.remove(id);
        }
        if !missing.is_empty() {
            tracing::info!(
                count = missing.len(),
                "unregistered downloads whose files are gone"
            );
        }
        Ok(Self {
            paths: Arc::new(RwLock::new(paths)),
            db: Some(db.clone()),
            ..Self::default()
        })
    }

    pub fn with_cache(mut self, cache: OfflineCache) -> Self {
        self.cache = cache;
        self
    }

    pub fn cache(&self) -> &OfflineCache {
        &self.cache
    }

    pub fn offline_mode(&self) -> bool {
        self.offline_mode.load(Ordering::SeqCst)
    }

    pub(crate) fn set_offline_mode(&self, on: bool) {
        self.offline_mode.store(on, Ordering::SeqCst);
        utils::http::set_offline(on);
    }

    /// Whether `item_id` has a downloaded or a cached copy.
    pub fn has_copy(&self, item_id: &str) -> bool {
        self.read().contains_key(item_id) || self.cache.contains(item_id)
    }

    /// Whether `track` plays without the internet: a file, or a server track with a copy here, or any server track when `source_needs_internet` is false; a radio stream never does.
    pub fn plays_offline(&self, track: &reader::Track, source_needs_internet: bool) -> bool {
        let uid = track.id.uid();
        match PlaybackItemRef::parse(&uid) {
            PlaybackItemRef::Local(_) => true,
            PlaybackItemRef::Radio { .. } => false,
            PlaybackItemRef::Server { item_id, .. } => {
                !source_needs_internet || self.has_copy(item_id)
            }
        }
    }

    /// The registered copy of `item_id`, whether or not the file is still there.
    pub fn path(&self, item_id: &str) -> Option<String> {
        self.read().get(item_id).cloned()
    }

    /// The copy of `item_id` if its file is still there; one that is gone is unregistered, so the listings that filter on downloads agree.
    pub fn present(&self, item_id: &str) -> Option<String> {
        let path = self.path(item_id)?;
        if Path::new(&path).is_file() {
            return Some(path);
        }
        self.forget_missing(item_id, &path);
        None
    }

    pub fn ids(&self) -> Vec<String> {
        self.read().keys().cloned().collect()
    }

    /// Signalled after a missing copy was unregistered, for whoever re-announces the listings.
    pub fn pruned(&self) -> Arc<Notify> {
        self.pruned.clone()
    }

    /// Register (`Some`) or forget (`None`) a copy; the map follows only once the row is written.
    pub async fn set(
        &self,
        db: &db::Db,
        item_id: &str,
        path: Option<String>,
    ) -> Result<(), ApiError> {
        let _writing = self.writer.lock().await;
        db.set_offline_track(item_id, path.as_deref())
            .await
            .map_err(|error| {
                ApiError::internal(format!("offline track registration failed: {error}"))
            })?;
        let mut paths = self.write();
        match path {
            Some(path) => paths.insert(item_id.to_string(), path),
            None => paths.remove(item_id),
        };
        Ok(())
    }

    fn forget_missing(&self, item_id: &str, path: &str) {
        {
            let mut paths = self.write();
            if paths.get(item_id).map(String::as_str) != Some(path) {
                return;
            }
            paths.remove(item_id);
        }
        tracing::info!(item = item_id, "unregistered a download whose file is gone");
        let (Some(db), Ok(runtime)) = (self.db.clone(), tokio::runtime::Handle::try_current())
        else {
            self.pruned.notify_one();
            return;
        };
        let index = self.clone();
        let item_id = item_id.to_string();
        runtime.spawn(async move {
            let _writing = index.writer.lock().await;
            // A copy registered again meanwhile is the newer truth.
            if index.read().contains_key(&item_id) {
                return;
            }
            if let Err(error) = db.set_offline_track(&item_id, None).await {
                tracing::warn!(%error, item = %item_id, "unregistering a missing download failed");
            }
            index.pruned.notify_one();
        });
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, HashMap<String, String>> {
        self.paths
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, HashMap<String, String>> {
        self.paths
            .write()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_download_whose_file_is_gone_is_unregistered_at_load_and_on_sight() {
        let dir = tempfile::tempdir().expect("tempdir");
        let database = db::init(&dir.path().join("offline.db")).await.expect("db");
        let kept = dir.path().join("kept.m4a");
        let doomed = dir.path().join("doomed.m4a");
        std::fs::write(&kept, b"audio").expect("write");
        std::fs::write(&doomed, b"audio").expect("write");
        for (id, path) in [
            ("kept", &kept),
            ("doomed", &doomed),
            ("gone", &dir.path().join("gone.m4a")),
        ] {
            database
                .set_offline_track(id, Some(&path.to_string_lossy()))
                .await
                .expect("register");
        }

        let index = OfflineIndex::load(&database).await.expect("load");
        assert_eq!(index.path("gone"), None, "pruned at load");
        assert!(
            !database
                .offline_tracks()
                .await
                .expect("rows")
                .contains_key("gone")
        );

        let pruned = index.pruned();
        std::fs::remove_file(&doomed).expect("delete");
        assert!(index.present("kept").is_some());
        assert_eq!(index.present("doomed"), None);
        assert_eq!(index.path("doomed"), None);
        pruned.notified().await;
        let rows = database.offline_tracks().await.expect("rows");
        assert!(!rows.contains_key("doomed"), "{rows:?}");
        assert!(rows.contains_key("kept"));
    }
}
