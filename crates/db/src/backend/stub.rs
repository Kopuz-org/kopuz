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

    async fn album_tracks(
        &self,
        _source: &crate::Source,
        _album_id: &str,
    ) -> Result<Vec<reader::Track>, DbError> {
        Ok(Vec::new())
    }

    async fn artist_tracks(
        &self,
        _source: &crate::Source,
        _artist: &str,
    ) -> Result<Vec<reader::Track>, DbError> {
        Ok(Vec::new())
    }

    async fn genre_tracks(
        &self,
        _source: &crate::Source,
        _genre: &str,
    ) -> Result<Vec<reader::Track>, DbError> {
        Ok(Vec::new())
    }

    async fn folder_tracks(&self, _prefix: &str) -> Result<Vec<reader::Track>, DbError> {
        Ok(Vec::new())
    }

    async fn recent_albums(
        &self,
        _source: &crate::Source,
        _limit: u32,
    ) -> Result<Vec<reader::Album>, DbError> {
        Ok(Vec::new())
    }

    async fn artist_sample_tracks(
        &self,
        _source: &crate::Source,
        _limit: u32,
    ) -> Result<Vec<reader::Track>, DbError> {
        Ok(Vec::new())
    }

    async fn top_genre(&self, _source: &crate::Source) -> Result<Option<String>, DbError> {
        Ok(None)
    }

    async fn search_corpus(&self, _source: &crate::Source) -> Result<Vec<reader::Track>, DbError> {
        Ok(Vec::new())
    }

    async fn tracks_by_keys(
        &self,
        _source: &crate::Source,
        _keys: &[String],
    ) -> Result<Vec<reader::Track>, DbError> {
        Ok(Vec::new())
    }

    async fn artists(&self, _source: &crate::Source) -> Result<Vec<(String, u32)>, DbError> {
        Ok(Vec::new())
    }

    async fn genres(&self, _source: &crate::Source) -> Result<Vec<String>, DbError> {
        Ok(Vec::new())
    }

    async fn album(
        &self,
        _source: &crate::Source,
        _album_id: &str,
    ) -> Result<Option<reader::Album>, DbError> {
        Ok(None)
    }

    async fn artist_images(
        &self,
    ) -> Result<
        (
            std::collections::HashMap<String, String>,
            std::collections::HashMap<String, std::path::PathBuf>,
            std::collections::HashMap<String, std::path::PathBuf>,
        ),
        DbError,
    > {
        Ok(Default::default())
    }

    async fn delete_tracks(
        &self,
        _source: &crate::Source,
        _keys: &[String],
    ) -> Result<u64, DbError> {
        Ok(0)
    }

    async fn delete_album(&self, _source: &crate::Source, _album_id: &str) -> Result<(), DbError> {
        Ok(())
    }

    async fn prune_source(
        &self,
        _source: &crate::Source,
        _keep_track_keys: &[String],
        _keep_album_ids: &[String],
    ) -> Result<(), DbError> {
        Ok(())
    }

    async fn set_artist_image(
        &self,
        _artist_norm: &str,
        _kind: &str,
        _image_ref: Option<&str>,
    ) -> Result<(), DbError> {
        Ok(())
    }

    async fn update_album_cover(
        &self,
        _source: &crate::Source,
        _album_id: &str,
        _cover_path: Option<&str>,
        _manual: bool,
    ) -> Result<(), DbError> {
        Ok(())
    }

    async fn upsert_playlist_meta(
        &self,
        _source: &crate::Source,
        _pl_id: &str,
        _name: &str,
        _cover_path: Option<&str>,
        _image_tag: Option<&str>,
    ) -> Result<(), DbError> {
        Ok(())
    }

    async fn delete_playlist(&self, _source: &crate::Source, _pl_id: &str) -> Result<(), DbError> {
        Ok(())
    }

    async fn set_playlist_tracks(
        &self,
        _source: &crate::Source,
        _pl_id: &str,
        _refs: &[String],
    ) -> Result<(), DbError> {
        Ok(())
    }

    async fn add_playlist_tracks(
        &self,
        _source: &crate::Source,
        _pl_id: &str,
        _refs: &[String],
    ) -> Result<(), DbError> {
        Ok(())
    }

    async fn remove_playlist_tracks(
        &self,
        _source: &crate::Source,
        _pl_id: &str,
        _refs: &[String],
    ) -> Result<(), DbError> {
        Ok(())
    }

    async fn create_folder(&self, _id: &str, _name: &str) -> Result<(), DbError> {
        Ok(())
    }

    async fn rename_folder(&self, _id: &str, _name: &str) -> Result<(), DbError> {
        Ok(())
    }

    async fn delete_folder(&self, _id: &str) -> Result<(), DbError> {
        Ok(())
    }

    async fn set_playlist_folder(
        &self,
        _playlist_ref: &str,
        _folder_id: Option<&str>,
    ) -> Result<(), DbError> {
        Ok(())
    }

    async fn bump_listen_count(&self, _track_uid: &str) -> Result<(), DbError> {
        Ok(())
    }

    async fn set_offline_track(&self, _id: &str, _path: Option<&str>) -> Result<(), DbError> {
        Ok(())
    }

    async fn albums(&self, _source: &crate::Source) -> Result<Vec<reader::Album>, DbError> {
        Ok(Vec::new())
    }

    async fn load_queue(&self) -> Result<crate::QueueSnapshot, DbError> {
        Ok(crate::QueueSnapshot::default())
    }

    async fn load_playlists(
        &self,
        _active_server: Option<&str>,
    ) -> Result<reader::PlaylistStore, DbError> {
        Ok(reader::PlaylistStore::default())
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

    async fn debug_reset(&self, _db_path: &std::path::Path) -> Result<(), DbError> {
        Ok(())
    }

    async fn debug_load_release(
        &self,
        _release_path: &std::path::Path,
        _db_path: &std::path::Path,
    ) -> Result<(), DbError> {
        Ok(())
    }

    async fn debug_seed_synthetic(&self, _n: u32) -> Result<(), DbError> {
        Ok(())
    }

    async fn debug_info(&self) -> Result<String, DbError> {
        Ok("wasm stub (in-memory)".to_string())
    }

    async fn debug_vacuum(&self) -> Result<(), DbError> {
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
