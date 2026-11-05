//! Catalog pages fetched this session, so going back to one draws what it showed instead of placeholders.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use api::{CatalogDetail, CatalogDetailRequest, CatalogItemKind};
use gpui_kit::{App, Global, Task};
use link::Link;

use crate::SourcesModel;

/// Past this a held page still draws, but is fetched again behind it.
const FRESH_FOR: Duration = Duration::from_secs(10 * 60);
const MAX_PAGES: usize = 64;
/// How long the pointer rests on a tile before its first page is fetched, so a play starts without the wait.
const DWELL: Duration = Duration::from_millis(250);

struct Entry {
    detail: CatalogDetail,
    fetched: Instant,
}

#[derive(Default)]
pub struct CatalogCache {
    pages: HashMap<String, Entry>,
    /// The tile the pointer rests on, and its dwell; dropping the task disarms it.
    armed: Option<(String, Task<()>)>,
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
        key_of(
            sources.read(cx).active().map(|source| source.id.as_str()),
            id,
        )
    }

    /// The source switch and key a fetch belongs to; `None` while a switch is settling.
    fn ticket(id: &str, cx: &App) -> Option<(u64, String)> {
        let epoch = SourcesModel::global(cx).read(cx).switch_epoch()?;
        Some((epoch, Self::key(id, cx)))
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
        Self::put_at(key, detail, cx);
    }

    fn put_at(key: String, detail: CatalogDetail, cx: &mut App) {
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

    /// The pointer came to rest on a tile: fetch its first page once it stays there a moment.
    pub fn arm(kind: CatalogItemKind, id: String, cx: &mut App) {
        if Self::get(&id, cx).is_some_and(|cached| cached.fresh) {
            return;
        }
        let Some(ticket) = Self::ticket(&id, cx) else {
            return;
        };
        let key = ticket.1.clone();
        let dwell = cx.spawn(async move |cx| {
            cx.background_executor().timer(DWELL).await;
            cx.update(|cx| {
                if Self::ticket(&id, cx).as_ref() == Some(&ticket) {
                    Self::prefetch(kind, id, ticket, cx);
                }
            });
        });
        cx.default_global::<Self>().armed = Some((key, dwell));
    }

    /// The pointer left before the dwell ran out; a fetch already under way still lands.
    pub fn disarm(id: &str, cx: &mut App) {
        let key = Self::key(id, cx);
        let cache = cx.default_global::<Self>();
        if cache.armed.as_ref().is_some_and(|(armed, _)| *armed == key) {
            cache.armed = None;
        }
    }

    /// A switch started since the ask may have reached the daemon first, so its answer could be another source's page.
    fn prefetch(kind: CatalogItemKind, id: String, ticket: (u64, String), cx: &mut App) {
        let request = CatalogDetailRequest::new(kind, id.clone());
        let call =
            Link::global(cx).call(move |api| async move { api.catalog_detail(request).await });
        cx.spawn(async move |cx| match call.await {
            Ok(detail) => cx.update(|cx| {
                if Self::ticket(&id, cx).as_ref() == Some(&ticket) {
                    Self::put_at(ticket.1, detail, cx);
                }
            }),
            Err(error) => tracing::debug!(%error, "prefetching a catalog page failed"),
        })
        .detach();
    }

    /// A page grew by a continuation: keep what was loaded, without making it look newly fetched.
    pub fn update(id: &str, detail: CatalogDetail, cx: &mut App) {
        let key = Self::key(id, cx);
        if let Some(entry) = cx.default_global::<Self>().pages.get_mut(&key) {
            entry.detail = detail;
        }
    }
}

fn key_of(source: Option<&str>, id: &str) -> String {
    format!("{}\u{0}{id}", source.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::key_of;

    #[test]
    fn the_same_id_from_two_sources_is_two_pages() {
        // A dwell armed on source A finds a different key once B is active, so it never fetches.
        let armed = key_of(Some("a"), "MPREb_1");
        assert_ne!(armed, key_of(Some("b"), "MPREb_1"));
        assert_eq!(armed, key_of(Some("a"), "MPREb_1"));
    }
}
