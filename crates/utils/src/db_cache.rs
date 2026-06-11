//! Process-wide handle to the app database for read-through caches living in
//! this crate (lyrics, metadata enrichment). Registered once at startup; every
//! cache degrades gracefully to fetch-only when unset (tests, early boot).

use std::sync::OnceLock;

static DB: OnceLock<db::Db> = OnceLock::new();

/// Register the database used by the persistent caches. Called once in `main`.
pub fn init(handle: db::Db) {
    let _ = DB.set(handle);
}

/// The registered database, if any. Public so caches in crates above `utils`
/// (e.g. discord-presence cover art) share the same handle.
pub fn get() -> Option<&'static db::Db> {
    DB.get()
}
