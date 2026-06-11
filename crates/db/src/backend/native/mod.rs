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
    #[allow(dead_code)] // wired up by the debug DB panel (step 12)
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

    async fn albums(&self, source: &crate::Source) -> Result<Vec<reader::Album>, DbError> {
        queries::albums(&self.pool(), source).await
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

    async fn prune_local_tracks(&self, root: &str, keep: &[String]) -> Result<u64, DbError> {
        writes::prune_local_tracks(&self.pool(), root, keep).await
    }
}
