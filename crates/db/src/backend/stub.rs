//! In-memory `Storage` stub for the wasm/web target (not a shipped target — no
//! persistence). Exists only so `dx build --platform web` compiles. Swap for a
//! `wa-sqlite` + OPFS backend if web ever ships (no call-site changes).

use std::sync::Mutex;

use crate::{DbError, Storage};

pub struct Stub {
    config: Mutex<Option<config::AppConfig>>,
}

impl Stub {
    pub fn new() -> Self {
        Self {
            config: Mutex::new(None),
        }
    }
}

#[async_trait::async_trait]
impl Storage for Stub {
    async fn load_config(&self) -> Result<Option<config::AppConfig>, DbError> {
        Ok(self.config.lock().unwrap().clone())
    }

    async fn save_config(&self, cfg: &config::AppConfig) -> Result<(), DbError> {
        *self.config.lock().unwrap() = Some(cfg.clone());
        Ok(())
    }

    async fn import_legacy_json(
        &self,
        _config_dir: &std::path::Path,
    ) -> Result<crate::ImportReport, DbError> {
        Ok(crate::ImportReport::default())
    }

    async fn finalize_migration(&self, _config_dir: &std::path::Path) -> Result<usize, DbError> {
        Ok(0)
    }

    async fn tracks_page(
        &self,
        _filter: &crate::TrackFilter,
        _page: crate::Page,
    ) -> Result<Vec<reader::Track>, DbError> {
        Ok(Vec::new())
    }

    async fn tracks_count(&self, _filter: &crate::TrackFilter) -> Result<u32, DbError> {
        Ok(0)
    }

    async fn albums(&self, _source: &crate::Source) -> Result<Vec<reader::Album>, DbError> {
        Ok(Vec::new())
    }

    async fn load_library(&self) -> Result<reader::Library, DbError> {
        Ok(reader::Library::default())
    }

    async fn load_queue(&self) -> Result<crate::QueueSnapshot, DbError> {
        Ok(crate::QueueSnapshot::default())
    }

    async fn load_playlists(&self) -> Result<reader::PlaylistStore, DbError> {
        Ok(reader::PlaylistStore::default())
    }

    async fn load_favorites_store(&self) -> Result<reader::FavoritesStore, DbError> {
        Ok(reader::FavoritesStore::default())
    }

    async fn save_library(&self, _lib: &reader::Library) -> Result<(), DbError> {
        Ok(())
    }

    async fn save_playlists(&self, _store: &reader::PlaylistStore) -> Result<(), DbError> {
        Ok(())
    }

    async fn save_favorites_store(
        &self,
        _store: &reader::FavoritesStore,
    ) -> Result<(), DbError> {
        Ok(())
    }

    async fn save_queue(&self, _snap: &crate::QueueSnapshot) -> Result<(), DbError> {
        Ok(())
    }

    async fn favorites(&self, _server_id: &str) -> Result<Vec<String>, DbError> {
        Ok(Vec::new())
    }

    async fn is_favorite(&self, _server_id: &str, _ref_: &str) -> Result<bool, DbError> {
        Ok(false)
    }

    async fn upsert_tracks(
        &self,
        _source: &crate::Source,
        _tracks: &[reader::Track],
    ) -> Result<(), DbError> {
        Ok(())
    }

    async fn upsert_albums(
        &self,
        _source: &crate::Source,
        _albums: &[reader::Album],
    ) -> Result<(), DbError> {
        Ok(())
    }

    async fn prune_local_tracks(&self, _root: &str, _keep: &[String]) -> Result<u64, DbError> {
        Ok(0)
    }

    async fn set_favorite(&self, _server_id: &str, _ref_: &str, _on: bool) -> Result<(), DbError> {
        Ok(())
    }

    async fn dirty_favorites(&self, _server_id: &str) -> Result<Vec<String>, DbError> {
        Ok(Vec::new())
    }

    async fn dirty_unlikes(&self, _server_id: &str) -> Result<Vec<String>, DbError> {
        Ok(Vec::new())
    }

    async fn load_server(&self, _id: &str) -> Result<Option<config::MusicServer>, DbError> {
        Ok(None)
    }

    async fn meta_get(&self, _cache_key: &str, _kind: &str) -> Result<Option<String>, DbError> {
        Ok(None)
    }

    async fn meta_put(&self, _cache_key: &str, _kind: &str, _payload: &str) -> Result<(), DbError> {
        Ok(())
    }

    async fn clear_favorite_dirty(&self, _server_id: &str, _ref_: &str) -> Result<(), DbError> {
        Ok(())
    }

    async fn replace_favorites_clean(
        &self,
        _server_id: &str,
        _refs: &[String],
    ) -> Result<(), DbError> {
        Ok(())
    }
}
