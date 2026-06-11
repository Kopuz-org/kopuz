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

/// The queue/progress snapshot, reconstructed from the `queue_state` row. The
/// in-memory `PersistedQueueState` (in the app crate) maps directly from this.
#[derive(Clone, Debug, Default)]
pub struct QueueSnapshot {
    pub version: u8,
    pub queue: Vec<reader::Track>,
    pub current_queue_index: usize,
    pub progress_secs: u64,
    pub shuffle_order: Vec<usize>,
    pub shuffle_enabled: bool,
}

/// Sort order for a track listing — maps to an indexed `ORDER BY`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TrackSort {
    /// Artist → album → disc → track (the natural library order).
    #[default]
    ArtistAlbum,
    Title,
    Artist,
    Album,
    /// Most-recently-added first (insertion order).
    DateAdded,
}

/// What a track listing selects: which source, how it's sorted, and an optional
/// case-insensitive search across title/artist/album. Drives `WHERE`/`ORDER BY`
/// so only the visible window is materialized.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackFilter {
    pub source: Source,
    pub sort: TrackSort,
    pub search: String,
}

impl TrackFilter {
    pub fn new(source: Source) -> Self {
        Self {
            source,
            sort: TrackSort::default(),
            search: String::new(),
        }
    }
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

    /// One window of a track listing (sorted + filtered in SQL — only this slice
    /// is materialized).
    async fn tracks_page(
        &self,
        filter: &TrackFilter,
        page: Page,
    ) -> Result<Vec<reader::Track>, DbError>;

    /// Total rows a `tracks_page` filter matches (for the scroll spacer).
    async fn tracks_count(&self, filter: &TrackFilter) -> Result<u32, DbError>;

    /// All albums for a source, ordered by artist then title.
    async fn albums(&self, source: &Source) -> Result<Vec<reader::Album>, DbError>;

    /// Reconstruct the full in-memory `Library` from the DB — local + active
    /// server tracks/albums, artist images, and the YT sync timestamps. Replaces
    /// the legacy `Library::load` from `library.json` (which can't parse the new
    /// `Track` shape); the DB is the converted source of truth.
    async fn load_library(&self) -> Result<reader::Library, DbError>;

    /// Reconstruct the queue/progress snapshot from the `queue_state` row.
    async fn load_queue(&self) -> Result<QueueSnapshot, DbError>;

    /// Reconstruct the in-memory `PlaylistStore` (local + ACTIVE-server
    /// playlists + folders) from the DB. Other servers' playlists stay in their
    /// rows, untouched — same per-server scoping as the library/favorites loads.
    async fn load_playlists(&self) -> Result<reader::PlaylistStore, DbError>;

    /// Reconstruct the in-memory `FavoritesStore` (local + active-server
    /// favorites) from the DB.
    async fn load_favorites_store(&self) -> Result<reader::FavoritesStore, DbError>;

    /// Persist the whole in-memory `Library` to the DB: upsert local +
    /// `active_server_id` tracks/albums, prune rows no longer present (scoped to
    /// local + that server only), replace artist images, and store the YT sync
    /// timestamps. The active id is passed by the caller (a consistent snapshot
    /// from the in-memory config) rather than re-read from the blob, so a
    /// concurrent server switch can't mis-scope the prune.
    async fn save_library(
        &self,
        lib: &reader::Library,
        active_server_id: Option<&str>,
    ) -> Result<(), DbError>;

    /// Replace the persisted playlists/folders for `'local'` + the active
    /// server with the in-memory store. Other servers' playlist rows are NEVER
    /// touched (the store only holds the active server's playlists).
    async fn save_playlists(
        &self,
        store: &reader::PlaylistStore,
        active_server_id: Option<&str>,
    ) -> Result<(), DbError>;

    /// Sync the in-memory favorites to the DB (local + active server).
    async fn save_favorites_store(
        &self,
        store: &reader::FavoritesStore,
        active_server_id: Option<&str>,
    ) -> Result<(), DbError>;

    /// The active server's tracks, albums, playlists, and favorites, freshly
    /// loaded for a server SWITCH: the in-memory caches are replaced with this
    /// instead of being cleared, so switching never destroys a server's cached
    /// rows and the previous server's cache survives for switching back.
    #[allow(clippy::type_complexity)]
    async fn load_server_cache(
        &self,
        server_id: &str,
    ) -> Result<
        (
            Vec<reader::Track>,
            Vec<reader::Album>,
            Vec<reader::models::JellyfinPlaylist>,
            Vec<String>,
        ),
        DbError,
    >;

    /// Persist the queue/progress snapshot to the single `queue_state` row.
    async fn save_queue(&self, snap: &QueueSnapshot) -> Result<(), DbError>;

    /// Hydrate one server row (creds included) into the in-memory shape — used
    /// by server switching so stored creds are reused instead of re-prompting.
    async fn load_server(&self, id: &str) -> Result<Option<config::MusicServer>, DbError>;

    /// Generic metadata-cache read (`metadata_cache` table): the `payload` for
    /// `(cache_key, kind)`, if cached.
    async fn meta_get(&self, cache_key: &str, kind: &str) -> Result<Option<String>, DbError>;

    /// Generic metadata-cache write (upsert of `payload` for `(cache_key, kind)`).
    async fn meta_put(&self, cache_key: &str, kind: &str, payload: &str) -> Result<(), DbError>;

    // --- Debug-panel operations (dev tooling; no-ops on the wasm stub) -----

    /// Delete the database files at `db_path`, re-init an empty schema there,
    /// and hot-swap the live pool onto it.
    async fn debug_reset(&self, db_path: &std::path::Path) -> Result<(), DbError>;

    /// Copy the release database over `db_path` (running any pending
    /// migrations on the copy) and hot-swap the live pool onto it.
    async fn debug_load_release(
        &self,
        release_path: &std::path::Path,
        db_path: &std::path::Path,
    ) -> Result<(), DbError>;

    /// Insert `n` synthetic local tracks (perf testing the windowed queries).
    async fn debug_seed_synthetic(&self, n: u32) -> Result<(), DbError>;

    /// Human-readable DB info: applied migrations + row counts.
    async fn debug_info(&self) -> Result<String, DbError>;

    /// VACUUM.
    async fn debug_vacuum(&self) -> Result<(), DbError>;

    /// The favorite refs (`track_key`s) for a server (`"local"` for filesystem).
    async fn favorites(&self, server_id: &str) -> Result<Vec<String>, DbError>;

    /// Whether `ref_` is favorited under `server_id`.
    async fn is_favorite(&self, server_id: &str, ref_: &str) -> Result<bool, DbError>;

    /// Toggle a favorite locally, optimistically. `on` upserts the row as a
    /// pending-like (`dirty=1`). `!on` deletes a never-pushed like outright and
    /// turns a synced row into a pending-unlike tombstone (`dirty=2`) so the
    /// removal can be pushed later. Works while unauthenticated — the reconciler
    /// flushes pending rows once a server is active.
    async fn set_favorite(&self, server_id: &str, ref_: &str, on: bool) -> Result<(), DbError>;

    /// Pending-like refs (`dirty=1`) not yet pushed to the server.
    async fn dirty_favorites(&self, server_id: &str) -> Result<Vec<String>, DbError>;

    /// Pending-unlike tombstones (`dirty=2`) not yet pushed to the server.
    async fn dirty_unlikes(&self, server_id: &str) -> Result<Vec<String>, DbError>;

    /// Resolve a ref after a successful remote push: a pending-like becomes
    /// clean, a pending-unlike tombstone is deleted.
    async fn clear_favorite_dirty(&self, server_id: &str, ref_: &str) -> Result<(), DbError>;

    /// Replace a server's favorites with the remote set (a sync pull): rows not in
    /// `refs` and not `dirty` are dropped, rows in `refs` are added clean. Dirty
    /// local rows are preserved (push-before-pull hasn't flushed them yet).
    async fn replace_favorites_clean(
        &self,
        server_id: &str,
        refs: &[String],
    ) -> Result<(), DbError>;

    /// Batch upsert tracks for a source (one transaction). Identity is
    /// `(source, track_key)`; an existing row is updated in place. Used by the
    /// streaming scan/sync so a batch lands atomically.
    async fn upsert_tracks(&self, source: &Source, tracks: &[reader::Track])
    -> Result<(), DbError>;

    /// Batch upsert albums for a source (one transaction).
    async fn upsert_albums(&self, source: &Source, albums: &[reader::Album])
    -> Result<(), DbError>;

    /// Delete local tracks whose path is under `root` but was not in the last
    /// scan (`keep` = the scanned `track_key`s). Returns rows removed. The scan's
    /// reconcile step, replacing the old post-scan `retain`.
    async fn prune_local_tracks(&self, root: &str, keep: &[String]) -> Result<u64, DbError>;
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

/// Blocking pre-boot read of the config blob — for the few values needed before
/// the app (and its async runtime/log subscriber) exists: the tracing toggle and
/// the titlebar mode. Opens the DB read-only without running migrations; `None`
/// if the DB or blob doesn't exist yet (first launch). Server/creds fields are
/// NOT hydrated — blob fields only.
#[cfg(not(target_arch = "wasm32"))]
pub fn peek_config(db_path: &std::path::Path) -> Option<config::AppConfig> {
    if !db_path.exists() {
        return None;
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    rt.block_on(async {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(db_path)
            .create_if_missing(false)
            .read_only(true);
        use sqlx::ConnectOptions;
        let mut conn = opts.connect().await.ok()?;
        let json: Option<String> =
            sqlx::query_scalar("SELECT json FROM app_config WHERE id = 1")
                .fetch_optional(&mut conn)
                .await
                .ok()
                .flatten();
        json.and_then(|j| serde_json::from_str(&j).ok())
    })
}

/// The RELEASE database path (`kopuz.db`), independent of build profile — the
/// debug panel's "load release DB" source.
#[cfg(not(target_arch = "wasm32"))]
pub fn release_db_path() -> std::path::PathBuf {
    config_dir().join("kopuz.db")
}

/// `<config_dir>` for kopuz (matches the legacy JSON store location).
#[cfg(not(target_arch = "wasm32"))]
pub fn config_dir() -> std::path::PathBuf {
    directories::ProjectDirs::from("com", "temidaradev", "kopuz")
        .map(|d| d.config_dir().to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("./config"))
}
