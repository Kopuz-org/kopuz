//! Catalog pages fetched this session, so going back to one renders what it showed instead of a spinner.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use dioxus::prelude::*;

/// Past this a cached page still renders, but is fetched again behind it.
const FRESH_FOR: Duration = Duration::from_secs(10 * 60);
const MAX_PAGES: usize = 64;

struct Entry {
    detail: api::CatalogDetail,
    fetched: Instant,
}

#[derive(Clone, Copy)]
pub struct CatalogCache {
    pages: Signal<HashMap<String, Entry>>,
    source: Memo<String>,
}

/// A cached page, and whether it is recent enough to skip the fetch.
pub struct Cached {
    pub detail: api::CatalogDetail,
    pub fresh: bool,
}

/// Provide the cache from the app root.
pub fn use_catalog_cache_provider() -> CatalogCache {
    let source = crate::use_db_queries::use_active_source();
    let cache = use_hook(|| CatalogCache {
        pages: Signal::new_in_scope(HashMap::new(), ScopeId::ROOT),
        source,
    });
    use_context_provider(|| cache)
}

pub fn use_catalog_cache() -> CatalogCache {
    use_context::<CatalogCache>()
}

impl CatalogCache {
    /// Keyed by the active source as well: two sources can hand out the same id.
    fn key(&self, kind: api::CatalogItemKind, id: &str) -> String {
        format!("{}\u{0}{kind:?}\u{0}{id}", self.source.peek())
    }

    /// Never subscribes the caller, so a fetch effect that reads it does not rerun on its own write.
    pub fn get(&self, kind: api::CatalogItemKind, id: &str) -> Option<Cached> {
        let key = self.key(kind, id);
        let pages = self.pages.peek();
        let entry = pages.get(&key)?;
        Some(Cached {
            detail: entry.detail.clone(),
            fresh: entry.fetched.elapsed() < FRESH_FOR,
        })
    }

    pub fn put(&self, kind: api::CatalogItemKind, id: &str, detail: api::CatalogDetail) {
        let key = self.key(kind, id);
        let mut pages = self.pages;
        let mut pages = pages.write();
        pages.insert(
            key,
            Entry {
                detail,
                fetched: Instant::now(),
            },
        );
        if pages.len() > MAX_PAGES
            && let Some(oldest) = pages
                .iter()
                .min_by_key(|(_, entry)| entry.fetched)
                .map(|(key, _)| key.clone())
        {
            pages.remove(&oldest);
        }
    }

    /// A page grew by a continuation: keep what was loaded, without making it look newly fetched.
    pub fn update(&self, kind: api::CatalogItemKind, id: &str, detail: api::CatalogDetail) {
        let key = self.key(kind, id);
        let mut pages = self.pages;
        if let Some(entry) = pages.write().get_mut(&key) {
            entry.detail = detail;
        }
    }
}
