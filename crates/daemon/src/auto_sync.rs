//! Syncs a remote source's library, playlists and favorites at startup and on source change when stale or empty.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use api::JobKind;
use tokio::sync::watch;

use crate::favorites::FavoritesService;
use crate::jobs::JobRunner;
use crate::library::LibraryService;
use crate::playlists::PlaylistService;

const STALE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

const KINDS: [JobKind; 3] = [
    JobKind::LibrarySync,
    JobKind::PlaylistSync,
    JobKind::FavoritesSync,
];

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

fn stamp_key(kind: JobKind) -> Option<&'static str> {
    match kind {
        JobKind::LibrarySync => Some("synced:library"),
        JobKind::PlaylistSync => Some("synced:playlists"),
        JobKind::FavoritesSync => Some("synced:favorites"),
        _ => None,
    }
}

/// Called by the sync jobs on success, so a sync the user started counts too.
pub async fn mark_synced(db: &db::Db, kind: JobKind, source: &config::Source) {
    let Some(key) = stamp_key(kind) else { return };
    if let Err(error) = db
        .meta_put(key, source.as_str(), &unix_now().to_string())
        .await
    {
        tracing::warn!(%error, ?kind, "could not record the sync time");
    }
}

async fn last_synced(db: &db::Db, kind: JobKind, source: &config::Source) -> Option<u64> {
    db.meta_get(stamp_key(kind)?, source.as_str())
        .await
        .ok()
        .flatten()
        .and_then(|raw| raw.parse().ok())
}

// An emptied or never-synced store may carry no stamp, so age alone cannot see it.
async fn is_empty(db: &db::Db, kind: JobKind, source: &config::Source) -> bool {
    match kind {
        JobKind::LibrarySync => db
            .tracks_page(
                &db::TrackFilter::new(source.clone()),
                db::Page {
                    offset: 0,
                    limit: 1,
                },
            )
            .await
            .map(|rows| rows.is_empty())
            .unwrap_or(false),
        JobKind::PlaylistSync => db
            .load_playlists(source)
            .await
            .map(|store| store.playlists.is_empty())
            .unwrap_or(false),
        JobKind::FavoritesSync => db
            .favorites(source.as_str())
            .await
            .map(|refs| refs.is_empty())
            .unwrap_or(false),
        _ => false,
    }
}

async fn is_due(db: &db::Db, kind: JobKind, source: &config::Source) -> bool {
    let stale = match last_synced(db, kind, source).await {
        Some(at) => unix_now().saturating_sub(at) >= STALE_AFTER.as_secs(),
        None => true,
    };
    stale || is_empty(db, kind, source).await
}

struct Syncs {
    db: db::Db,
    jobs: Arc<JobRunner>,
    library: Arc<LibraryService>,
    playlists: Arc<PlaylistService>,
    favorites: Arc<FavoritesService>,
}

impl Syncs {
    async fn check(&self, config: &config::AppConfig) {
        // A local library has nothing to pull; it has the file scan.
        let active = server::source::active(self.db.clone(), config);
        if !active.capabilities().sync {
            return;
        }
        let source = &config.active_source;
        for kind in KINDS {
            if !is_due(&self.db, kind, source).await {
                continue;
            }
            let started = match kind {
                JobKind::LibrarySync => self.library.spawn_remote_sync(&self.jobs),
                JobKind::PlaylistSync => self.playlists.spawn_sync(&self.jobs),
                JobKind::FavoritesSync => self.favorites.spawn_sync(&self.jobs),
                _ => continue,
            };
            match started {
                Ok(_) => tracing::info!(?kind, source = source.as_str(), "auto-sync started"),
                // Usually a sync of this kind is already running.
                Err(error) => tracing::debug!(%error, ?kind, "auto-sync not started"),
            }
        }
    }
}

/// Check once now, then again whenever the active source changes.
pub fn spawn(
    db: db::Db,
    jobs: Arc<JobRunner>,
    library: Arc<LibraryService>,
    playlists: Arc<PlaylistService>,
    favorites: Arc<FavoritesService>,
    mut config: watch::Receiver<config::AppConfig>,
) {
    let syncs = Syncs {
        db,
        jobs,
        library,
        playlists,
        favorites,
    };
    tokio::spawn(async move {
        let first = config.borrow_and_update().clone();
        let mut current = first.active_source.clone();
        syncs.check(&first).await;
        while config.changed().await.is_ok() {
            let next = config.borrow_and_update().clone();
            if next.active_source != current {
                current = next.active_source.clone();
                syncs.check(&next).await;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{is_due, mark_synced, stamp_key};
    use api::JobKind;

    fn server() -> config::Source {
        config::Source::Server("srv-1".into())
    }

    async fn store() -> (db::Db, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = db::init(&dir.path().join("auto-sync.db"))
            .await
            .expect("db init");
        (db, dir)
    }

    async fn stock(db: &db::Db) {
        let track = reader::Track {
            id: reader::TrackId::Server {
                service: config::MusicService::YtMusic,
                item_id: "v1".into(),
            },
            cover: None,
            album_id: String::new(),
            title: "t".into(),
            artist: "a".into(),
            album: String::new(),
            duration: 60,
            khz: 44,
            bitrate: 320,
            track_number: None,
            disc_number: None,
            musicbrainz_release_id: None,
            musicbrainz_recording_id: None,
            musicbrainz_track_id: None,
            playlist_item_id: None,
            artists: vec!["a".into()],
            credits: Vec::new(),
        };
        db.upsert_tracks(&server(), &[track]).await.expect("upsert");
    }

    #[tokio::test]
    async fn a_source_never_synced_is_due() {
        let (db, _dir) = store().await;
        stock(&db).await;
        assert!(is_due(&db, JobKind::LibrarySync, &server()).await);
    }

    #[tokio::test]
    async fn a_fresh_sync_of_a_stocked_store_is_not_due() {
        let (db, _dir) = store().await;
        stock(&db).await;
        mark_synced(&db, JobKind::LibrarySync, &server()).await;
        assert!(!is_due(&db, JobKind::LibrarySync, &server()).await);
    }

    /// The cutover migration empties a store its stamp still calls fresh.
    #[tokio::test]
    async fn an_empty_store_is_due_however_recent_its_sync() {
        let (db, _dir) = store().await;
        mark_synced(&db, JobKind::LibrarySync, &server()).await;
        assert!(is_due(&db, JobKind::LibrarySync, &server()).await);
    }

    #[tokio::test]
    async fn a_sync_older_than_a_day_is_due() {
        let (db, _dir) = store().await;
        stock(&db).await;
        let two_days_ago = super::unix_now() - 2 * 24 * 60 * 60;
        db.meta_put(
            stamp_key(JobKind::LibrarySync).unwrap(),
            server().as_str(),
            &two_days_ago.to_string(),
        )
        .await
        .unwrap();
        assert!(is_due(&db, JobKind::LibrarySync, &server()).await);
    }

    /// A stamp belongs to one source; another stays due until it syncs itself.
    #[tokio::test]
    async fn a_stamp_does_not_carry_to_another_source() {
        let (db, _dir) = store().await;
        stock(&db).await;
        mark_synced(&db, JobKind::LibrarySync, &server()).await;
        let other = config::Source::Server("srv-2".into());
        assert!(is_due(&db, JobKind::LibrarySync, &other).await);
    }
}
