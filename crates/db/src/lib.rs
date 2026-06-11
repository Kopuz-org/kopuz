//! Kopuz persistence layer (issue #347).
//!
//! Owns the SQLite schema and all persistence behind a single async [`Storage`]
//! trait. Native targets implement it with sqlx; wasm (not a shipped target)
//! gets a thin in-memory stub so the build stays green. Everything above this
//! crate (reactive hooks, UI) is driver-agnostic.
//!
//! Dependency direction: `db` sits ABOVE `config`/`reader` (it persists their
//! types), so those crates stay pure model definitions and all save/load lives
//! here.

use std::sync::Arc;

mod backend;

/// What a one-shot legacy-JSON import did. `ran == false` means it was skipped
/// (already migrated, or no legacy JSON present); the counts are then all zero.
#[derive(Debug, Default, Clone)]
pub struct ImportReport {
    pub ran: bool,
    pub tracks: usize,
    pub albums: usize,
    pub playlists: usize,
    pub favorites: usize,
    pub servers: usize,
}

/// Where a track/playlist/favorite comes from: the local filesystem, or a
/// specific media server (by its `servers.id`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum Source {
    #[default]
    Local,
    Server(String),
}

impl Source {
    /// The `source` column value: `"local"` or the server id.
    pub fn as_str(&self) -> &str {
        match self {
            Source::Local => "local",
            Source::Server(id) => id.as_str(),
        }
    }
}

/// A window into a list query (for virtual-scrolled big lists).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Page {
    pub offset: u32,
    pub limit: u32,
}

/// Errors surfaced by the storage layer. String-wrapped so the type is identical
/// on native and wasm (sqlx isn't compiled for wasm).
#[derive(Debug, Clone)]
pub enum DbError {
    Backend(String),
    Serde(String),
    Io(String),
}

impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DbError::Backend(e) => write!(f, "db backend: {e}"),
            DbError::Serde(e) => write!(f, "db serde: {e}"),
            DbError::Io(e) => write!(f, "db io: {e}"),
        }
    }
}

impl std::error::Error for DbError {}

impl From<serde_json::Error> for DbError {
    fn from(e: serde_json::Error) -> Self {
        DbError::Serde(e.to_string())
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<sqlx::Error> for DbError {
    fn from(e: sqlx::Error) -> Self {
        DbError::Backend(e.to_string())
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<sqlx::migrate::MigrateError> for DbError {
    fn from(e: sqlx::migrate::MigrateError) -> Self {
        DbError::Backend(e.to_string())
    }
}

/// The persistence API. One impl per target (sqlx native / in-mem stub). Grows
/// as the migration lands more domains; this is the foundation slice.
#[async_trait::async_trait]
pub trait Storage: Send + Sync {
    /// Load the persisted `AppConfig` (the single-row JSON blob), or `None` if
    /// the app has never been configured.
    async fn load_config(&self) -> Result<Option<config::AppConfig>, DbError>;

    /// Persist the whole `AppConfig` as the single-row JSON blob.
    async fn save_config(&self, cfg: &config::AppConfig) -> Result<(), DbError>;

    /// One-shot import of the legacy `*.json` store at `config_dir` into the DB,
    /// then rename each imported file to `*.json.bak` and drop a sentinel. No-op
    /// if the DB already holds data or the sentinel exists. Idempotent; safe to
    /// call on every launch. (Native only; the wasm stub no-ops.)
    async fn import_legacy_json(
        &self,
        config_dir: &std::path::Path,
    ) -> Result<ImportReport, DbError>;

    /// Point of no return: rename each imported `X.json` → `X.json.bak` (kept for
    /// downgrade). Call only once every domain reads from the DB. Idempotent;
    /// no-op until a real import has happened. Returns how many files moved.
    async fn finalize_migration(&self, config_dir: &std::path::Path) -> Result<usize, DbError>;
}

/// Cheap-`Clone` handle to the active storage backend, shared via Dioxus context.
#[derive(Clone)]
pub struct Db(Arc<dyn Storage>);

impl std::ops::Deref for Db {
    type Target = dyn Storage;
    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

/// Open the database and apply migrations (native), or build the in-memory stub
/// (wasm). Native callers should `block_on` this in `main()` before mounting.
#[cfg(not(target_arch = "wasm32"))]
pub async fn init(db_path: &std::path::Path) -> Result<Db, DbError> {
    let native = backend::native::Native::open(db_path).await?;
    Ok(Db(Arc::new(native)))
}

/// wasm: an in-memory stub so `dx build --platform web` compiles. Not persistent.
#[cfg(target_arch = "wasm32")]
pub fn init_stub() -> Db {
    Db(Arc::new(backend::stub::Stub::new()))
}

/// The on-disk database path: `KOPUZ_DB_PATH` override, else `<config_dir>/kopuz.db`
/// (release) or `kopuz-debug.db` (debug builds, so `dx run` never touches real data).
#[cfg(not(target_arch = "wasm32"))]
pub fn default_db_path() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("KOPUZ_DB_PATH") {
        return std::path::PathBuf::from(p);
    }
    let name = if cfg!(debug_assertions) {
        "kopuz-debug.db"
    } else {
        "kopuz.db"
    };
    config_dir().join(name)
}

/// `<config_dir>` for kopuz (matches the legacy JSON store location).
#[cfg(not(target_arch = "wasm32"))]
pub fn config_dir() -> std::path::PathBuf {
    directories::ProjectDirs::from("com", "temidaradev", "kopuz")
        .map(|d| d.config_dir().to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("./config"))
}
