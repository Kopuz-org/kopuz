//! Native sqlx + SQLite backend. Owns the pool (behind `ArcSwap` so debug tools
//! can hot-swap the DB) and runs migrations. SQL lives here, grouped by domain
//! as the migration lands more methods.

use std::path::Path;
use std::sync::Arc;

use arc_swap::ArcSwap;
use sqlx::SqlitePool;
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous,
};

use crate::{DbError, Storage};

mod cfg_store;
mod dump;
mod migrate;
mod queries;
mod rows;
mod writes;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

pub struct Native {
    pool: ArcSwap<SqlitePool>,
}

impl Native {
    /// Open (creating if needed) the DB at `path`, snapshot before any pending
    /// migration, then apply migrations.
    pub async fn open(path: &Path) -> Result<Self, DbError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| DbError::Io(e.to_string()))?;
        }
        snapshot_if_pending(path).await;
        let pool = open_pool(path).await?;
        MIGRATOR.run(&pool).await?;
        Ok(Self {
            pool: ArcSwap::from_pointee(pool),
        })
    }

    fn pool(&self) -> Arc<SqlitePool> {
        self.pool.load_full()
    }

    /// Rebind to a different pool (debug "load release DB" / "reset"). Live.
    pub fn swap_pool(&self, pool: SqlitePool) {
        self.pool.store(Arc::new(pool));
    }
}

async fn open_pool(path: &Path) -> Result<SqlitePool, DbError> {
    let opts = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(std::time::Duration::from_secs(5))
        .foreign_keys(true);
    SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await
        .map_err(Into::into)
}

/// Before applying new migrations to an existing DB, copy it (plus WAL sidecars)
/// to `<db>.pre-<applied_version>.bak` so a downgrade can restore it. Best-effort.
async fn snapshot_if_pending(path: &Path) {
    if !path.exists() {
        return; // fresh DB, nothing to snapshot
    }
    let Ok(pool) = open_pool(path).await else {
        return;
    };
    // Max applied version (the table won't exist on a pre-migration legacy DB).
    let applied: Option<i64> =
        sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
            .fetch_one(&pool)
            .await
            .unwrap_or(None);
    let available = MIGRATOR.iter().map(|m| m.version).max();
    let pending = match (applied, available) {
        (Some(a), Some(v)) => v > a,
        (None, Some(_)) => false, // fresh/just-created DB with no migrations yet → not a downgrade risk
        _ => false,
    };
    pool.close().await;
    if !pending {
        return;
    }
    let stamp = applied.unwrap_or(0);
    for ext in ["", "-wal", "-shm"] {
        let src = with_ext(path, ext);
        if src.exists() {
            let dst = backup_name(path, stamp, ext);
            if let Err(e) = std::fs::copy(&src, &dst) {
                tracing::warn!(error = %e, src = %src.display(), "db: pre-migration snapshot failed");
            }
        }
    }
    tracing::info!(applied = stamp, "db: snapshotted before applying pending migrations");
}

fn with_ext(path: &Path, suffix: &str) -> std::path::PathBuf {
    if suffix.is_empty() {
        path.to_path_buf()
    } else {
        let mut s = path.as_os_str().to_os_string();
        s.push(suffix);
        std::path::PathBuf::from(s)
    }
}

fn backup_name(path: &Path, stamp: i64, suffix: &str) -> std::path::PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(format!(".pre-{stamp}.bak{suffix}"));
    std::path::PathBuf::from(s)
}

#[async_trait::async_trait]
impl Storage for Native {
    async fn load_config(&self) -> Result<Option<config::AppConfig>, DbError> {
        cfg_store::load_config(&self.pool()).await
    }

    async fn save_config(&self, cfg: &config::AppConfig) -> Result<(), DbError> {
        cfg_store::save_config(&self.pool(), cfg).await
    }

    async fn import_legacy_json(
        &self,
        config_dir: &Path,
    ) -> Result<crate::ImportReport, DbError> {
        migrate::run_json_import(&self.pool(), config_dir).await
    }

    async fn finalize_migration(&self, config_dir: &Path) -> Result<usize, DbError> {
        migrate::finalize_migration(&self.pool(), config_dir).await
    }

    async fn tracks_page(
        &self,
        filter: &crate::TrackFilter,
        page: crate::Page,
    ) -> Result<Vec<reader::Track>, DbError> {
        queries::tracks_page(&self.pool(), filter, page).await
    }

    async fn tracks_count(&self, filter: &crate::TrackFilter) -> Result<u32, DbError> {
        queries::tracks_count(&self.pool(), filter).await
    }

    async fn album_tracks(
        &self,
        source: &crate::Source,
        album_id: &str,
    ) -> Result<Vec<reader::Track>, DbError> {
        queries::album_tracks(&self.pool(), source, album_id).await
    }

    async fn artist_tracks(
        &self,
        source: &crate::Source,
        artist: &str,
    ) -> Result<Vec<reader::Track>, DbError> {
        queries::artist_tracks(&self.pool(), source, artist).await
    }

    async fn genre_tracks(
        &self,
        source: &crate::Source,
        genre: &str,
    ) -> Result<Vec<reader::Track>, DbError> {
        queries::genre_tracks(&self.pool(), source, genre).await
    }

    async fn folder_tracks(&self, prefix: &str) -> Result<Vec<reader::Track>, DbError> {
        queries::folder_tracks(&self.pool(), prefix).await
    }

    async fn recent_albums(
        &self,
        source: &crate::Source,
        limit: u32,
    ) -> Result<Vec<reader::Album>, DbError> {
        queries::recent_albums(&self.pool(), source, limit).await
    }

    async fn artist_sample_tracks(
        &self,
        source: &crate::Source,
        limit: u32,
    ) -> Result<Vec<reader::Track>, DbError> {
        queries::artist_sample_tracks(&self.pool(), source, limit).await
    }

    async fn top_genre(&self, source: &crate::Source) -> Result<Option<String>, DbError> {
        queries::top_genre(&self.pool(), source).await
    }

    async fn search_corpus(&self, source: &crate::Source) -> Result<Vec<reader::Track>, DbError> {
        queries::search_corpus(&self.pool(), source).await
    }

    async fn tracks_by_keys(
        &self,
        source: &crate::Source,
        keys: &[String],
    ) -> Result<Vec<reader::Track>, DbError> {
        queries::tracks_by_keys(&self.pool(), source, keys).await
    }

    async fn artists(&self, source: &crate::Source) -> Result<Vec<(String, u32)>, DbError> {
        queries::artists(&self.pool(), source).await
    }

    async fn genres(&self, source: &crate::Source) -> Result<Vec<String>, DbError> {
        queries::genres(&self.pool(), source).await
    }

    async fn album(
        &self,
        source: &crate::Source,
        album_id: &str,
    ) -> Result<Option<reader::Album>, DbError> {
        queries::album(&self.pool(), source, album_id).await
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
        dump::artist_images(&self.pool()).await
    }

    async fn delete_tracks(&self, source: &crate::Source, keys: &[String]) -> Result<u64, DbError> {
        writes::delete_tracks(&self.pool(), source, keys).await
    }

    async fn delete_album(&self, source: &crate::Source, album_id: &str) -> Result<(), DbError> {
        writes::delete_album(&self.pool(), source, album_id).await
    }

    async fn prune_source(
        &self,
        source: &crate::Source,
        keep_track_keys: &[String],
        keep_album_ids: &[String],
    ) -> Result<(), DbError> {
        writes::prune_source(&self.pool(), source, keep_track_keys, keep_album_ids).await
    }

    async fn set_artist_image(
        &self,
        artist_norm: &str,
        kind: &str,
        image_ref: Option<&str>,
    ) -> Result<(), DbError> {
        writes::set_artist_image(&self.pool(), artist_norm, kind, image_ref).await
    }

    async fn update_album_cover(
        &self,
        source: &crate::Source,
        album_id: &str,
        cover_path: Option<&str>,
        manual: bool,
    ) -> Result<(), DbError> {
        writes::update_album_cover(&self.pool(), source, album_id, cover_path, manual).await
    }

    async fn upsert_playlist_meta(
        &self,
        source: &crate::Source,
        pl_id: &str,
        name: &str,
        cover_path: Option<&str>,
        image_tag: Option<&str>,
    ) -> Result<(), DbError> {
        writes::upsert_playlist_meta(&self.pool(), source, pl_id, name, cover_path, image_tag)
            .await
    }

    async fn delete_playlist(&self, source: &crate::Source, pl_id: &str) -> Result<(), DbError> {
        writes::delete_playlist(&self.pool(), source, pl_id).await
    }

    async fn set_playlist_tracks(
        &self,
        source: &crate::Source,
        pl_id: &str,
        refs: &[String],
    ) -> Result<(), DbError> {
        writes::set_playlist_tracks(&self.pool(), source, pl_id, refs).await
    }

    async fn set_folders(
        &self,
        folders: &[reader::models::PlaylistFolder],
    ) -> Result<(), DbError> {
        writes::set_folders(&self.pool(), folders).await
    }

    async fn bump_listen_count(&self, track_uid: &str) -> Result<(), DbError> {
        cfg_store::bump_listen_count(&self.pool(), track_uid).await
    }

    async fn set_offline_track(&self, id: &str, path: Option<&str>) -> Result<(), DbError> {
        writes::set_offline_track(&self.pool(), id, path).await
    }

    async fn albums(&self, source: &crate::Source) -> Result<Vec<reader::Album>, DbError> {
        queries::albums(&self.pool(), source).await
    }

    async fn load_queue(&self) -> Result<crate::QueueSnapshot, DbError> {
        dump::load_queue(&self.pool()).await
    }

    async fn load_playlists(
        &self,
        active_server: Option<&str>,
    ) -> Result<reader::PlaylistStore, DbError> {
        dump::load_playlists(&self.pool(), active_server).await
    }

    async fn save_queue(&self, snap: &crate::QueueSnapshot) -> Result<(), DbError> {
        writes::save_queue(&self.pool(), snap).await
    }

    async fn favorites(&self, server_id: &str) -> Result<Vec<String>, DbError> {
        queries::favorites(&self.pool(), server_id).await
    }

    async fn is_favorite(&self, server_id: &str, ref_: &str) -> Result<bool, DbError> {
        queries::is_favorite(&self.pool(), server_id, ref_).await
    }

    async fn upsert_tracks(
        &self,
        source: &crate::Source,
        tracks: &[reader::Track],
    ) -> Result<(), DbError> {
        writes::upsert_tracks(&self.pool(), source, tracks).await
    }

    async fn upsert_albums(
        &self,
        source: &crate::Source,
        albums: &[reader::Album],
    ) -> Result<(), DbError> {
        writes::upsert_albums(&self.pool(), source, albums).await
    }

    async fn set_favorite(&self, server_id: &str, ref_: &str, on: bool) -> Result<(), DbError> {
        writes::set_favorite(&self.pool(), server_id, ref_, on).await
    }

    async fn dirty_favorites(&self, server_id: &str) -> Result<Vec<String>, DbError> {
        writes::dirty_favorites(&self.pool(), server_id).await
    }

    async fn dirty_unlikes(&self, server_id: &str) -> Result<Vec<String>, DbError> {
        writes::dirty_unlikes(&self.pool(), server_id).await
    }

    async fn load_server(&self, id: &str) -> Result<Option<config::MusicServer>, DbError> {
        cfg_store::load_server(&self.pool(), id).await
    }

    async fn meta_get(&self, cache_key: &str, kind: &str) -> Result<Option<String>, DbError> {
        writes::meta_get(&self.pool(), cache_key, kind).await
    }

    async fn meta_put(&self, cache_key: &str, kind: &str, payload: &str) -> Result<(), DbError> {
        writes::meta_put(&self.pool(), cache_key, kind, payload).await
    }

    async fn debug_reset(&self, db_path: &Path) -> Result<(), DbError> {
        self.pool().close().await;
        for ext in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(with_ext(db_path, ext));
        }
        let pool = open_pool(db_path).await?;
        MIGRATOR.run(&pool).await?;
        self.swap_pool(pool);
        Ok(())
    }

    async fn debug_load_release(
        &self,
        release_path: &Path,
        db_path: &Path,
    ) -> Result<(), DbError> {
        if !release_path.exists() {
            return Err(DbError::Io(format!(
                "release db not found at {}",
                release_path.display()
            )));
        }
        self.pool().close().await;
        for ext in ["", "-wal", "-shm"] {
            let src = with_ext(release_path, ext);
            let dst = with_ext(db_path, ext);
            let _ = std::fs::remove_file(&dst);
            if src.exists() {
                std::fs::copy(&src, &dst).map_err(|e| DbError::Io(e.to_string()))?;
            }
        }
        let pool = open_pool(db_path).await?;
        MIGRATOR.run(&pool).await?;
        self.swap_pool(pool);
        Ok(())
    }

    async fn debug_seed_synthetic(&self, n: u32) -> Result<(), DbError> {
        let pool = self.pool();
        let mut tx = pool.begin().await?;
        for i in 0..n {
            let key = format!("/synthetic/{i:06}.flac");
            let title = format!("Synthetic {i:06}");
            let artist = format!("Artist {:03}", i % 100);
            let album = format!("Album {:04}", i % 2000);
            sqlx::query(
                "INSERT OR IGNORE INTO tracks (source, track_key, path, title, artist, album, artists_json) \
                 VALUES ('local', ?1, ?1, ?2, ?3, ?4, '[]')",
            )
            .bind(&key)
            .bind(&title)
            .bind(&artist)
            .bind(&album)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn debug_info(&self) -> Result<String, DbError> {
        let pool = self.pool();
        let migrations: Vec<(i64, String)> = sqlx::query_as(
            "SELECT version, description FROM _sqlx_migrations ORDER BY version",
        )
        .fetch_all(&*pool)
        .await?;
        let mut out = String::new();
        for (v, d) in &migrations {
            out.push_str(&format!("migration {v} — {d}\n"));
        }
        for table in ["tracks", "albums", "playlists", "favorites", "servers", "metadata_cache"] {
            let n: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
                .fetch_one(&*pool)
                .await?;
            out.push_str(&format!("{table}: {n}\n"));
        }
        Ok(out)
    }

    async fn debug_vacuum(&self) -> Result<(), DbError> {
        sqlx::query("VACUUM").execute(&*self.pool()).await?;
        Ok(())
    }

    async fn clear_favorite_dirty(&self, server_id: &str, ref_: &str) -> Result<(), DbError> {
        writes::clear_favorite_dirty(&self.pool(), server_id, ref_).await
    }

    async fn replace_favorites_clean(
        &self,
        server_id: &str,
        refs: &[String],
    ) -> Result<(), DbError> {
        writes::replace_favorites_clean(&self.pool(), server_id, refs).await
    }
}
