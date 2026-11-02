//! Finding artist photos, which the daemon does itself: one bulk listing from a `Library` server, one lookup by id per linked artist on a `Remote` catalog (YT), with a definite miss remembered for a day.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use api::{ApiError, Table};
use server::source::{ActiveSource, ArtistView};

use super::{LibraryService, db_error};

const MISS_KIND: &str = db::ARTIST_PHOTO_MISS_KIND;
const MISS_TTL_SECS: i64 = 86_400;
/// Enough to fill a grid page quickly without hammering the catalog.
const WORKERS: usize = 6;
/// Photos found mid-batch show up this often at most.
const ANNOUNCE_EVERY: Duration = Duration::from_secs(2);
/// A boot or a source switch sweeps a source no more often than this.
const SWEEP_SPACING: Duration = Duration::from_secs(10 * 60);
/// What each worker of a sweep waits between two lookups, since nobody is looking at the grid it fills.
const SWEEP_LOOKUP_GAP: Duration = Duration::from_millis(250);
/// A sweep at boot waits this long, so it does not compete with the startup reads.
const BOOT_SWEEP_DELAY: Duration = Duration::from_secs(20);

/// Which artists one refresh looks up.
enum Wanted {
    /// Every artist still missing a photo, paced: a background sweep.
    Missing,
    /// These keys, now: a caller asked.
    Keys(HashSet<String>),
}

impl LibraryService {
    /// Find photos for these artists if the library has none for them and no fresh miss; the keys only scope a per-artist search.
    pub async fn refresh_artist_artwork(&self, artists: Vec<String>) -> Result<(), ApiError> {
        self.refresh_photos(Wanted::Keys(artists.into_iter().collect()))
            .await
    }

    /// Sweep the active source shortly after boot and whenever it changes, until the session ends.
    pub fn spawn_artist_photo_upkeep(self: &Arc<Self>) {
        let Some(mut sources) = self.sources_rx.get().cloned() else {
            return;
        };
        let service = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(BOOT_SWEEP_DELAY).await;
            let mut swept: Option<config::Source> = None;
            loop {
                let active = sources.borrow_and_update().active.clone();
                if swept.as_ref() != Some(&active) {
                    swept = Some(active);
                    service.sweep_artist_photos(false).await;
                }
                if sources.changed().await.is_err() {
                    return;
                }
            }
        });
    }

    /// Look up every artist of the active source still missing a photo; `fresh` says a sync just stored new ones, which no spacing holds back.
    pub(super) async fn sweep_artist_photos(&self, fresh: bool) {
        if self.offline().offline_mode() {
            return;
        }
        let scope = self.current_sources().active.as_str().to_string();
        let due = match self.photo_sweeps.lock() {
            Ok(mut sweeps) => claim_sweep(&mut sweeps, scope, fresh, Instant::now()),
            Err(_) => false,
        };
        if !due {
            return;
        }
        if let Err(error) = self.refresh_photos(Wanted::Missing).await {
            tracing::debug!(%error, "artist photo sweep failed");
        }
    }

    async fn refresh_photos(&self, wanted: Wanted) -> Result<(), ApiError> {
        let sources = self.current_sources();
        let source: ActiveSource = Arc::from(server::source::active(self.db.clone(), &sources));
        match source.capabilities().artist_view {
            ArtistView::Library => self.refresh_bulk(&source).await,
            ArtistView::Remote => {
                let scope = source.source();
                let rows = self.db.artists(scope).await.map_err(db_error)?;
                let linked = lookups(rows, &wanted, scope);
                let gap = matches!(wanted, Wanted::Missing).then_some(SWEEP_LOOKUP_GAP);
                self.refresh_each(&source, linked, gap).await
            }
        }
    }

    /// One listing for the whole server; only a photo or name that differs from the stored one is written and announced.
    async fn refresh_bulk(&self, source: &ActiveSource) -> Result<(), ApiError> {
        let listed = source.fetch_artists().await.unwrap_or_default();
        if listed.is_empty() {
            return Ok(());
        }
        let mut changed = match source.name_listed_artists(&listed).await {
            Ok(renamed) => renamed > 0,
            Err(error) => {
                tracing::warn!(%error, "could not name the listed artists");
                false
            }
        };
        let scope = source.source().as_str().to_string();
        let (_, photos) = self.db.artist_images().await.map_err(db_error)?;
        let linked = self
            .db
            .linked_artist_keys(source.source())
            .await
            .map_err(db_error)?;
        for (key, url) in changed_photos(listed, &scope, &photos, &linked) {
            changed |= source
                .set_artist_image(&key, "server", Some(&url))
                .await
                .is_ok();
        }
        if changed {
            self.invalidate(Table::Artists);
        }
        Ok(())
    }

    /// One search per artist, a few at a time, each worker waiting `gap` between two.
    async fn refresh_each(
        &self,
        source: &ActiveSource,
        artists: Vec<reader::ArtistCredit>,
        gap: Option<Duration>,
    ) -> Result<(), ApiError> {
        let (_, photos) = self.db.artist_images().await.map_err(db_error)?;
        let fresh_misses: std::collections::HashSet<String> = self
            .db
            .meta_keys_since(MISS_KIND, MISS_TTL_SECS)
            .await
            .unwrap_or_default()
            .into_iter()
            .collect();

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
        // Announced as they land, on a timer, so a long batch neither holds every photo to the end nor floods the frontends.
        let found = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let last_announced = Arc::new(Mutex::new(Instant::now()));
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
                                session.invalidate(Table::Artists);
                            }
                        }
                        if let Some(gap) = gap {
                            tokio::time::sleep(gap).await;
                        }
                    }
                }
            })
            .collect();
        futures_util::future::join_all(workers).await;
        // A last one, in case the tail of the batch landed inside a window the frontend had already coalesced away.
        if found.load(std::sync::atomic::Ordering::Relaxed) > 0 {
            self.invalidate(Table::Artists);
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

/// The wanted artists a catalog can look up: only those it issued an id for, since a credit's text may name a stranger.
fn lookups(
    rows: Vec<db::ArtistRow>,
    wanted: &Wanted,
    scope: &config::Source,
) -> Vec<reader::ArtistCredit> {
    rows.into_iter()
        .filter(|row| match wanted {
            Wanted::Missing => true,
            Wanted::Keys(keys) => keys.contains(&row.key),
        })
        .filter_map(|row| {
            let id = row.source_id?;
            Some(reader::ArtistCredit {
                name: row.name,
                id: Some(id),
                source: Some(scope.clone()),
                key: Some(row.key),
            })
        })
        .collect()
}

/// The key a source's listing credit is filed under: its own, else its linked row by the id the source issued; a bare name files nowhere.
pub(super) fn credit_key(
    credit: &reader::ArtistCredit,
    linked: &std::collections::HashMap<String, String>,
) -> Option<String> {
    match (&credit.key, &credit.id) {
        (Some(key), _) => Some(key.clone()),
        (None, Some(id)) => linked.get(id).cloned(),
        (None, None) => None,
    }
}

/// Whether a sweep of `scope` may run now, marking it as run when it may.
fn claim_sweep(
    sweeps: &mut std::collections::HashMap<String, Instant>,
    scope: String,
    fresh: bool,
    now: Instant,
) -> bool {
    let recent = sweeps
        .get(&scope)
        .is_some_and(|at| now.saturating_duration_since(*at) < SWEEP_SPACING);
    if recent && !fresh {
        return false;
    }
    sweeps.insert(scope, now);
    true
}

/// The photos of a server's listing that differ from what `scope` already stores, by artist key.
fn changed_photos(
    listed: Vec<(reader::ArtistCredit, Option<String>)>,
    scope: &str,
    photos: &std::collections::HashMap<(String, String), reader::ArtistImageRef>,
    linked: &std::collections::HashMap<String, String>,
) -> Vec<(String, String)> {
    listed
        .into_iter()
        .filter_map(|(artist, url)| {
            let url = url?;
            let key = credit_key(&artist, linked)?;
            let stored = photos.get(&(scope.to_string(), key.clone()));
            (stored != Some(&reader::ArtistImageRef::Remote(url.clone()))).then_some((key, url))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn a_listing_rewrites_only_the_photos_that_changed() {
        let photos = HashMap::from([
            (
                ("srv".to_string(), "ada".to_string()),
                reader::ArtistImageRef::Remote("https://srv/ada.jpg".into()),
            ),
            (
                ("srv".to_string(), "boris".to_string()),
                reader::ArtistImageRef::Remote("https://srv/old.jpg".into()),
            ),
        ]);
        let linked = HashMap::from([
            ("id-ada".to_string(), "ada".to_string()),
            ("id-boris".to_string(), "boris".to_string()),
            ("id-cleo".to_string(), "cleo".to_string()),
            ("id-dora".to_string(), "dora".to_string()),
        ]);
        let listing = |id: &str, url: Option<&str>| {
            (
                reader::ArtistCredit {
                    name: id.into(),
                    id: Some(id.into()),
                    source: None,
                    key: None,
                },
                url.map(str::to_string),
            )
        };

        let changed = changed_photos(
            vec![
                listing("id-ada", Some("https://srv/ada.jpg")),
                listing("id-boris", Some("https://srv/boris.jpg")),
                listing("id-cleo", Some("https://srv/cleo.jpg")),
                listing("id-nobody", Some("https://srv/nobody.jpg")),
                listing("id-dora", None),
            ],
            "srv",
            &photos,
            &linked,
        );

        assert_eq!(
            changed,
            vec![
                ("boris".to_string(), "https://srv/boris.jpg".to_string()),
                ("cleo".to_string(), "https://srv/cleo.jpg".to_string()),
            ]
        );
    }

    #[test]
    fn a_source_is_swept_again_only_after_the_spacing_or_a_sync() {
        let mut sweeps = HashMap::new();
        let start = Instant::now();
        assert!(claim_sweep(&mut sweeps, "yt".into(), false, start));
        assert!(!claim_sweep(
            &mut sweeps,
            "yt".into(),
            false,
            start + Duration::from_secs(60)
        ));
        assert!(
            claim_sweep(&mut sweeps, "srv".into(), false, start),
            "each source on its own"
        );
        assert!(claim_sweep(
            &mut sweeps,
            "yt".into(),
            true,
            start + Duration::from_secs(60)
        ));
        let later = start + Duration::from_secs(60) + SWEEP_SPACING;
        assert!(claim_sweep(&mut sweeps, "yt".into(), false, later));
    }

    fn row(key: &str, source_id: Option<&str>, name: &str) -> db::ArtistRow {
        db::ArtistRow {
            key: key.into(),
            source_id: source_id.map(Into::into),
            name: name.into(),
            tracks: 1,
            albums: 0,
        }
    }

    #[test]
    fn an_unlinked_artist_is_never_looked_up() {
        let scope = config::Source::Server("yt".into());
        let rows = || {
            vec![
                row("UC-teto", Some("UC-teto"), "TETO KASANE"),
                row("9f2b", None, "____natural / TETO KASANE"),
            ]
        };

        let swept = lookups(rows(), &Wanted::Missing, &scope);
        let asked = lookups(
            rows(),
            &Wanted::Keys(HashSet::from(["9f2b".to_string()])),
            &scope,
        );

        assert_eq!(
            swept
                .iter()
                .map(|credit| credit.id.as_deref())
                .collect::<Vec<_>>(),
            [Some("UC-teto")]
        );
        assert!(asked.is_empty(), "asking for it by key searches nothing");
    }

    #[test]
    fn a_listing_credit_with_only_a_name_files_under_no_artist() {
        let linked = HashMap::from([("id-ada".to_string(), "ada".to_string())]);

        assert_eq!(
            credit_key(&reader::ArtistCredit::unlinked("Ada"), &linked),
            None
        );
        let by_id = reader::ArtistCredit {
            key: None,
            ..reader::ArtistCredit::linked("Ada", "id-ada")
        };
        assert_eq!(credit_key(&by_id, &linked).as_deref(), Some("ada"));
    }
}
