//! Catalog pages fetched this session, so going back to one draws what it showed instead of placeholders.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use api::CatalogDetail;
use gpui_kit::{App, Global};

use crate::SourcesModel;

/// Past this a held page still draws, but is fetched again behind it.
const FRESH_FOR: Duration = Duration::from_secs(10 * 60);
const MAX_PAGES: usize = 64;

struct Entry {
    detail: CatalogDetail,
    fetched: Instant,
}

#[derive(Default)]
pub struct CatalogCache {
    pages: HashMap<String, Entry>,
}

impl Global for CatalogCache {}

/// A held page, and whether it is recent enough to skip the fetch.
pub struct Cached {
    pub detail: CatalogDetail,
    pub fresh: bool,
}

impl CatalogCache {
    /// Keyed by the active source as well: two sources can hand out the same id.
    fn key(id: &str, cx: &App) -> String {
        let sources = SourcesModel::global(cx);
        let source = sources.read(cx).active().map(|source| source.id.as_str());
        format!("{}\u{0}{id}", source.unwrap_or_default())
    }

    pub fn get(id: &str, cx: &App) -> Option<Cached> {
        let key = Self::key(id, cx);
        let entry = cx.try_global::<Self>()?.pages.get(&key)?;
        Some(Cached {
            detail: entry.detail.clone(),
            fresh: entry.fetched.elapsed() < FRESH_FOR,
        })
    }

    /// The line over a held page's first shelf, such as the signed-in name over YouTube's feed.
    pub fn strapline(id: &str, cx: &App) -> Option<String> {
        let key = Self::key(id, cx);
        let entry = cx.try_global::<Self>()?.pages.get(&key)?;
        entry.detail.shelves.first()?.strapline.clone()
    }

    pub fn put(id: &str, detail: CatalogDetail, cx: &mut App) {
        let key = Self::key(id, cx);
        let pages = &mut cx.default_global::<Self>().pages;
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
    pub fn update(id: &str, detail: CatalogDetail, cx: &mut App) {
        let key = Self::key(id, cx);
        if let Some(entry) = cx.default_global::<Self>().pages.get_mut(&key) {
            entry.detail = detail;
        }
    }
}
