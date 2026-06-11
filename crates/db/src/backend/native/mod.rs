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

mod migrate;

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
        let pool = self.pool();
        let json: Option<String> = sqlx::query_scalar!("SELECT json FROM app_config WHERE id = 1")
            .fetch_optional(&*pool)
            .await?;
        match json {
            Some(j) => Ok(Some(serde_json::from_str(&j)?)),
            None => Ok(None),
        }
    }

    async fn save_config(&self, cfg: &config::AppConfig) -> Result<(), DbError> {
        let json = serde_json::to_string(cfg)?;
        let pool = self.pool();
        sqlx::query!(
            "INSERT INTO app_config (id, json) VALUES (1, ?1) \
             ON CONFLICT(id) DO UPDATE SET json = ?1",
            json
        )
        .execute(&*pool)
        .await?;
        Ok(())
    }

    async fn import_legacy_json(
        &self,
        config_dir: &Path,
    ) -> Result<crate::ImportReport, DbError> {
        migrate::run_json_import(&self.pool(), config_dir).await
    }
}
