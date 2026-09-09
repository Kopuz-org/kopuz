//! Finding artist photos.
//!
//! Two shapes, chosen by the source's capability rather than its identity: a
//! `Library` server answers with one bulk listing, a `Remote` catalog (YT) has
//! to be asked per artist. Both write what they find into the library, so the
//! next open resolves instantly instead of searching again.
//!
//! A definitive "no photo exists" is recorded too, with a day's TTL. Without
//! that, every visit re-searches the same artists -- most of a local library's
//! artists have no photo anywhere, and the search is a network round trip
//! each.
//!
//! This ran in the app, holding results in a session map the grid read
//! alongside the persisted one. It writes only to the library now: the daemon
//! announces the change and every frontend re-reads, so a photo found while
//! one window was open shows up in the others too.

use std::sync::{Arc, Mutex};

use api::{ApiError, Table};
use server::source::{ActiveSource, ArtistView};
use utils::artist::normalize_artist_key;

use super::{LibraryService, db_error};

const MISS_KIND: &str = "artist_photo_miss";
const MISS_TTL_SECS: i64 = 86_400;
/// Enough to fill a grid page quickly without hammering the catalog.
const WORKERS: usize = 6;

impl LibraryService {
    /// Find photos for `names` that the library does not already have one for.
    ///
    /// Names already resolved, and names whose miss is still fresh, are
    /// skipped -- so calling this on every page open is cheap once the first
    /// pass has run.
    pub async fn refresh_artist_artwork(&self, names: Vec<String>) -> Result<(), ApiError> {
        let config = self.current_config();
        let source: ActiveSource = Arc::from(server::source::active(self.db.clone(), &config));
        match source.capabilities().artist_view {
            ArtistView::Library => self.refresh_bulk(&source).await,
            ArtistView::Remote => self.refresh_each(&source, names).await,
        }
    }

    /// One listing for the whole server.
    async fn refresh_bulk(&self, source: &ActiveSource) -> Result<(), ApiError> {
        let images = source.fetch_artist_images().await.unwrap_or_default();
        if images.is_empty() {
            return Ok(());
        }
        for (name, url) in images {
            let _ = source
                .set_artist_image(&normalize_artist_key(&name), "server", Some(&url))
                .await;
        }
        self.invalidate(Table::Tracks);
        Ok(())
    }

    /// One search per artist, a few at a time.
    async fn refresh_each(
        &self,
        source: &ActiveSource,
        names: Vec<String>,
    ) -> Result<(), ApiError> {
        let (_, photos) = self.db.artist_images().await.map_err(db_error)?;
        let fresh_misses: std::collections::HashSet<String> = self
            .db
            .meta_keys_since(MISS_KIND, MISS_TTL_SECS)
            .await
            .unwrap_or_default()
            .into_iter()
            .collect();
        let pending: Vec<String> = names
            .into_iter()
            .filter(|name| {
                let key = normalize_artist_key(name);
                !photos.contains_key(&key) && !fresh_misses.contains(&key)
            })
            .collect();
        if pending.is_empty() {
            return Ok(());
        }

        let queue = Arc::new(Mutex::new(pending.into_iter()));
        // Announce as they land rather than once at the end: a grid of a few
        // hundred artists takes a while to search, and holding every photo
        // until the last one resolves is a page of placeholders for all of it.
        // The frontend coalesces these, so announcing often is cheap.
        let found = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let workers: Vec<_> = (0..WORKERS)
            .map(|_| {
                let source = source.clone();
                let queue = queue.clone();
                let found = found.clone();
                let session = self.session.get().cloned();
                async move {
                    while let Some(name) = queue.lock().ok().and_then(|mut names| names.next()) {
                        if resolve_one(&source, &name).await {
                            found.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            if let Some(session) = &session {
                                session.invalidate(Table::Tracks);
                            }
                        }
                    }
                }
            })
            .collect();
        futures_util::future::join_all(workers).await;
        // A last one, in case the tail of the batch landed inside a window the
        // frontend had already coalesced away.
        if found.load(std::sync::atomic::Ordering::Relaxed) > 0 {
            self.invalidate(Table::Tracks);
        }
        Ok(())
    }
}

/// Answers whether a photo was found and stored.
async fn resolve_one(source: &ActiveSource, name: &str) -> bool {
    let key = normalize_artist_key(name);
    match source.fetch_artist_image(name).await {
        Ok(Some(url)) => {
            let _ = source.set_artist_image(&key, "server", Some(&url)).await;
            true
        }
        // A definitive miss is worth remembering; a transient error is not,
        // since it would hide the artist for a whole day over a blip.
        Ok(None) => {
            let _ = source.set_meta(&key, MISS_KIND, "").await;
            false
        }
        Err(error) => {
            tracing::debug!(%error, artist = name, "artist photo lookup failed");
            false
        }
    }
}
