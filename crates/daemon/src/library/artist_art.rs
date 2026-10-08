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

use super::{LibraryService, db_error};

const MISS_KIND: &str = db::ARTIST_PHOTO_MISS_KIND;
const MISS_TTL_SECS: i64 = 86_400;
/// Enough to fill a grid page quickly without hammering the catalog.
const WORKERS: usize = 6;
/// Photos found mid-batch show up this often at most.
const ANNOUNCE_EVERY: std::time::Duration = std::time::Duration::from_secs(2);

impl LibraryService {
    /// Find photos for the artists the library does not already have one for.
    ///
    /// Artists already resolved, and those whose miss is still fresh, are
    /// skipped -- so calling this on every page open is cheap once the first
    /// pass has run.
    pub async fn refresh_artist_artwork(&self, artists: Vec<String>) -> Result<(), ApiError> {
        let config = self.current_config();
        let source: ActiveSource = Arc::from(server::source::active(self.db.clone(), &config));
        match source.capabilities().artist_view {
            ArtistView::Library => self.refresh_bulk(&source).await,
            ArtistView::Remote => {
                let wanted: std::collections::HashSet<&str> =
                    artists.iter().map(String::as_str).collect();
                let scope = source.source();
                // A search wants the name the library calls them, which only the listing holds.
                let named = self
                    .db
                    .artists(scope)
                    .await
                    .map_err(db_error)?
                    .into_iter()
                    .filter(|row| wanted.contains(row.key.as_str()))
                    .map(|row| reader::ArtistCredit {
                        name: row.name,
                        id: row.source_id,
                        source: Some(scope.clone()),
                        key: Some(row.key),
                    })
                    .collect();
                self.refresh_each(&source, named).await
            }
        }
    }

    /// One listing for the whole server.
    async fn refresh_bulk(&self, source: &ActiveSource) -> Result<(), ApiError> {
        let images = source.fetch_artist_images().await.unwrap_or_default();
        if images.is_empty() {
            return Ok(());
        }
        let unlinked = self
            .db
            .unlinked_artist_keys(source.source())
            .await
            .map_err(db_error)?;
        let linked = self
            .db
            .linked_artist_keys(source.source())
            .await
            .map_err(db_error)?;
        for (artist, url) in images {
            let Some(key) = credit_key(&artist, &linked, &unlinked) else {
                continue;
            };
            let _ = source.set_artist_image(&key, "server", Some(&url)).await;
        }
        self.invalidate(Table::Tracks);
        Ok(())
    }

    /// One search per artist, a few at a time.
    async fn refresh_each(
        &self,
        source: &ActiveSource,
        artists: Vec<reader::ArtistCredit>,
    ) -> Result<(), ApiError> {
        let (_, photos) = self.db.artist_images().await.map_err(db_error)?;
        let fresh_misses: std::collections::HashSet<String> = self
            .db
            .meta_keys_since(MISS_KIND, MISS_TTL_SECS)
            .await
            .unwrap_or_default()
            .into_iter()
            .collect();
        // The same lookup names a linked artist, so one wearing a credit's text is looked up though it has a photo.
        let unnamed = self
            .db
            .artist_keys_unnamed_by_source(source.source())
            .await
            .map_err(db_error)?;
        let scope = source.source().clone();
        let mut claim = Claim {
            in_flight: self.artwork_in_flight.clone(),
            keys: Vec::new(),
        };
        let pending: Vec<reader::ArtistCredit> = {
            let Ok(mut in_flight) = claim.in_flight.lock() else {
                return Ok(());
            };
            artists
                .into_iter()
                .filter(|artist| {
                    let Some(key) = artist.key.as_deref() else {
                        return false;
                    };
                    let miss = db::artist_miss_name(&scope, key);
                    let has_photo =
                        photos.contains_key(&(scope.as_str().to_string(), key.to_string()));
                    let wants_photo = !has_photo && !fresh_misses.contains(&miss);
                    let wanted = wants_photo || unnamed.contains(key);
                    wanted && in_flight.insert(miss.clone()) && {
                        claim.keys.push(miss);
                        true
                    }
                })
                .collect()
        };
        if pending.is_empty() {
            return Ok(());
        }

        let queue = Arc::new(Mutex::new(pending.into_iter()));
        // Announce as they land rather than once at the end: a grid of a few
        // hundred artists takes a while to search, and holding every photo
        // until the last one resolves is a page of placeholders for all of it.
        // Every announcement re-runs every track-keyed read in every frontend, so a
        // batch that finds a photo every few milliseconds announces on a timer.
        let found = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let last_announced = Arc::new(Mutex::new(std::time::Instant::now()));
        let workers: Vec<_> = (0..WORKERS)
            .map(|_| {
                let source = source.clone();
                let queue = queue.clone();
                let found = found.clone();
                let last_announced = last_announced.clone();
                let session = self.session.get().cloned();
                async move {
                    while let Some(artist) = queue.lock().ok().and_then(|mut queue| queue.next()) {
                        if resolve_one(&source, &artist).await {
                            found.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            if let Some(session) = &session
                                && announce_due(&last_announced)
                            {
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

/// The artists one batch is looking up, released when it finishes or is dropped part way.
struct Claim {
    in_flight: Arc<Mutex<std::collections::HashSet<String>>>,
    keys: Vec<String>,
}

impl Drop for Claim {
    fn drop(&mut self) {
        if let Ok(mut in_flight) = self.in_flight.lock() {
            for key in &self.keys {
                in_flight.remove(key);
            }
        }
    }
}

/// Whether a batch in progress may announce its photos again, restarting the wait when it may.
fn announce_due(last: &Mutex<std::time::Instant>) -> bool {
    let Ok(mut last) = last.lock() else {
        return false;
    };
    if last.elapsed() < ANNOUNCE_EVERY {
        return false;
    }
    *last = std::time::Instant::now();
    true
}

/// Answers whether the lookup stored anything the grid shows: a photo, or the name the source gives the artist.
async fn resolve_one(source: &ActiveSource, artist: &reader::ArtistCredit) -> bool {
    let Some(key) = artist.key.as_deref() else {
        return false;
    };
    let found = match source.fetch_artist_image(artist).await {
        Ok(found) => found,
        // A transient error isn't remembered, since a miss would hide the artist for a whole day over a blip.
        Err(error) => {
            tracing::debug!(%error, artist = %artist.name, "artist photo lookup failed");
            return false;
        }
    };
    let renamed = match (artist.id.as_deref(), found.name.as_deref()) {
        (Some(id), Some(name)) => source.name_artist(id, name).await.unwrap_or(false),
        _ => false,
    };
    match found.image {
        Some(url) => {
            let _ = source.set_artist_image(key, "server", Some(&url)).await;
            true
        }
        None => {
            let miss = db::artist_miss_name(source.source(), key);
            let _ = source.set_meta(&miss, MISS_KIND, "").await;
            renamed
        }
    }
}

/// The key a source's listing credit is filed under: its own, else its linked row by the id the source issued, else its unlinked row by name.
pub(super) fn credit_key(
    credit: &reader::ArtistCredit,
    linked: &std::collections::HashMap<String, String>,
    unlinked: &std::collections::HashMap<String, String>,
) -> Option<String> {
    match (&credit.key, &credit.id) {
        (Some(key), _) => Some(key.clone()),
        (None, Some(id)) => linked.get(id).cloned(),
        (None, None) => unlinked
            .get(&utils::artist::normalize_artist_key(&credit.name))
            .cloned(),
    }
}
