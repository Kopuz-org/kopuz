//! Syncs a remote source's library, playlists and favorites at startup and whenever the configured interval has passed.

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use api::JobKind;
use tokio::sync::watch;
use tokio::time::Instant;

use crate::favorites::FavoritesService;
use crate::jobs::{JobRunner, Trigger};
use crate::library::LibraryService;
use crate::playlists::PlaylistService;

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

pub(crate) async fn last_synced(
    db: &db::Db,
    kind: JobKind,
    source: &config::Source,
) -> Option<u64> {
    db.meta_get(stamp_key(kind)?, source.as_str())
        .await
        .ok()
        .flatten()
        .and_then(|raw| raw.parse().ok())
}

/// How often the schedule runs; `None` when the setting is 0 (never).
pub(crate) fn interval(config: &config::AppConfig) -> Option<Duration> {
    match config.sync_interval_minutes {
        0 => None,
        minutes => Some(Duration::from_secs(u64::from(minutes) * 60)),
    }
}

/// Time left before a sync of `kind` is due again; zero when it is due now.
async fn until_due(
    db: &db::Db,
    kind: JobKind,
    source: &config::Source,
    interval: Duration,
) -> Duration {
    match last_synced(db, kind, source).await {
        Some(at) => interval.saturating_sub(Duration::from_secs(unix_now().saturating_sub(at))),
        None => Duration::ZERO,
    }
}

/// What a check found: the kinds due now, and how long until the earliest next one.
#[derive(Debug, PartialEq, Eq)]
struct Plan {
    due: Vec<JobKind>,
    next: Option<Duration>,
}

/// A due kind is about to run, so its next one is a full interval away.
async fn plan(db: &db::Db, source: &config::Source, interval: Option<Duration>) -> Plan {
    let Some(interval) = interval else {
        return Plan {
            due: Vec::new(),
            next: None,
        };
    };
    let mut due = Vec::new();
    let mut next: Option<Duration> = None;
    for kind in KINDS {
        let left = until_due(db, kind, source, interval).await;
        let wait = if left.is_zero() {
            due.push(kind);
            interval
        } else {
            left
        };
        next = Some(next.map_or(wait, |soonest| soonest.min(wait)));
    }
    Plan { due, next }
}

struct Syncs {
    db: db::Db,
    jobs: Arc<JobRunner>,
    library: Arc<LibraryService>,
    playlists: Arc<PlaylistService>,
    favorites: Arc<FavoritesService>,
}

impl Syncs {
    /// Start what is due; the answer is how long to wait before looking again.
    async fn check(&self, config: &config::AppConfig) -> Option<Duration> {
        // A local library has nothing to pull; it has the file scan.
        let active = server::source::active(self.db.clone(), config);
        if !active.capabilities().sync {
            return None;
        }
        let source = &config.active_source;
        let plan = plan(&self.db, source, interval(config)).await;
        // A playlist sync pulls favorites first, so a favorites sync due beside it would pull them twice.
        let playlists_due = plan.due.contains(&JobKind::PlaylistSync);
        for kind in plan.due {
            if playlists_due && kind == JobKind::FavoritesSync {
                continue;
            }
            let started = match kind {
                JobKind::LibrarySync => self
                    .library
                    .spawn_remote_sync(&self.jobs, Trigger::Schedule),
                JobKind::PlaylistSync => self.playlists.spawn_sync(&self.jobs, Trigger::Schedule),
                JobKind::FavoritesSync => self.favorites.spawn_sync(&self.jobs, Trigger::Schedule),
                _ => continue,
            };
            match started {
                Ok(_) => tracing::info!(?kind, source = source.as_str(), "auto-sync started"),
                // Usually a sync of this kind is already running.
                Err(error) => tracing::debug!(%error, ?kind, "auto-sync not started"),
            }
        }
        plan.next
    }
}

/// Sleeps to `deadline`, or forever when there is none.
pub(crate) async fn until(deadline: Option<Instant>) {
    match deadline {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

/// Run `check` now, when its own answer says it is time, and whenever the source or interval changes.
async fn drive<F, Fut>(mut config: watch::Receiver<config::AppConfig>, mut check: F)
where
    F: FnMut(config::AppConfig) -> Fut,
    Fut: Future<Output = Option<Duration>>,
{
    let first = config.borrow_and_update().clone();
    let mut armed = (first.active_source.clone(), first.sync_interval_minutes);
    let mut deadline = check(first).await.map(|wait| Instant::now() + wait);
    loop {
        tokio::select! {
            changed = config.changed() => {
                if changed.is_err() {
                    return;
                }
                let next = config.borrow_and_update().clone();
                let key = (next.active_source.clone(), next.sync_interval_minutes);
                if key != armed {
                    armed = key;
                    deadline = check(next).await.map(|wait| Instant::now() + wait);
                }
            }
            () = until(deadline) => {
                let current = config.borrow().clone();
                deadline = check(current).await.map(|wait| Instant::now() + wait);
            }
        }
    }
}

/// Sync at startup if due, then again each time the interval passes or the source or interval changes.
pub fn spawn(
    db: db::Db,
    jobs: Arc<JobRunner>,
    library: Arc<LibraryService>,
    playlists: Arc<PlaylistService>,
    favorites: Arc<FavoritesService>,
    config: watch::Receiver<config::AppConfig>,
) {
    let syncs = Arc::new(Syncs {
        db,
        jobs,
        library,
        playlists,
        favorites,
    });
    tokio::spawn(drive(config, move |config| {
        let syncs = syncs.clone();
        async move { syncs.check(&config).await }
    }));
}
#[cfg(test)]
mod tests {
    use super::{Plan, drive, interval, mark_synced, plan, stamp_key, until_due};
    use api::JobKind;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tokio::sync::watch;

    const DAY: Duration = Duration::from_secs(24 * 60 * 60);

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
            replay_gain: config::ReplayGainInfo::default(),
            credits: Vec::new(),
        };
        db.upsert_tracks(&server(), &[track]).await.expect("upsert");
    }

    async fn stamp_ago(db: &db::Db, kind: JobKind, ago: Duration) {
        let at = super::unix_now() - ago.as_secs();
        db.meta_put(stamp_key(kind).unwrap(), server().as_str(), &at.to_string())
            .await
            .unwrap();
    }

    #[test]
    fn the_default_interval_is_the_old_day() {
        assert_eq!(interval(&config::AppConfig::default()), Some(DAY));
    }

    #[test]
    fn zero_minutes_is_no_interval() {
        let config = config::AppConfig {
            sync_interval_minutes: 0,
            ..Default::default()
        };
        assert_eq!(interval(&config), None);
    }

    #[tokio::test]
    async fn a_source_never_synced_is_due() {
        let (db, _dir) = store().await;
        stock(&db).await;
        let found = plan(&db, &server(), Some(DAY)).await;
        assert_eq!(
            found,
            Plan {
                due: vec![
                    JobKind::LibrarySync,
                    JobKind::PlaylistSync,
                    JobKind::FavoritesSync
                ],
                next: Some(DAY),
            }
        );
    }

    #[tokio::test]
    async fn a_fresh_sync_of_a_stocked_store_is_not_due() {
        let (db, _dir) = store().await;
        stock(&db).await;
        mark_synced(&db, JobKind::LibrarySync, &server()).await;
        let found = plan(&db, &server(), Some(DAY)).await;
        assert!(!found.due.contains(&JobKind::LibrarySync));
        assert!(found.due.contains(&JobKind::PlaylistSync));
        let left = until_due(&db, JobKind::LibrarySync, &server(), DAY).await;
        assert!(left > DAY - Duration::from_secs(60) && left <= DAY);
    }

    #[tokio::test]
    async fn a_sync_older_than_the_interval_is_due() {
        let (db, _dir) = store().await;
        stock(&db).await;
        stamp_ago(&db, JobKind::LibrarySync, 2 * DAY).await;
        assert!(
            plan(&db, &server(), Some(DAY))
                .await
                .due
                .contains(&JobKind::LibrarySync)
        );
        assert!(
            !plan(&db, &server(), Some(3 * DAY))
                .await
                .due
                .contains(&JobKind::LibrarySync)
        );
    }

    /// The next wake is the soonest unsynced kind's expiry, not a fixed poll.
    #[tokio::test]
    async fn the_next_wake_is_when_the_soonest_sync_expires() {
        let (db, _dir) = store().await;
        let hour = Duration::from_secs(60 * 60);
        for (kind, ago) in [
            (JobKind::LibrarySync, hour),
            (JobKind::PlaylistSync, 5 * hour),
            (JobKind::FavoritesSync, 2 * hour),
        ] {
            stamp_ago(&db, kind, ago).await;
        }
        let found = plan(&db, &server(), Some(DAY)).await;
        assert!(found.due.is_empty());
        let next = found.next.expect("something is pending");
        assert!(next <= 19 * hour && next > 19 * hour - Duration::from_secs(60));
    }

    /// A shorter interval makes the same stamp due, which is how a changed setting takes effect.
    #[tokio::test]
    async fn a_shorter_interval_makes_a_fresh_stamp_due() {
        let (db, _dir) = store().await;
        stamp_ago(&db, JobKind::LibrarySync, Duration::from_secs(2 * 60 * 60)).await;
        let day = plan(&db, &server(), Some(DAY)).await;
        assert!(!day.due.contains(&JobKind::LibrarySync));
        let hour = plan(&db, &server(), Some(Duration::from_secs(60 * 60))).await;
        assert!(hour.due.contains(&JobKind::LibrarySync));
    }

    #[tokio::test]
    async fn a_zero_interval_starts_nothing_and_schedules_nothing() {
        let (db, _dir) = store().await;
        stock(&db).await;
        let found = plan(&db, &server(), None).await;
        assert_eq!(
            found,
            Plan {
                due: Vec::new(),
                next: None
            }
        );
    }

    /// A stamp belongs to one source; another stays due until it syncs itself.
    #[tokio::test]
    async fn a_stamp_does_not_carry_to_another_source() {
        let (db, _dir) = store().await;
        stock(&db).await;
        mark_synced(&db, JobKind::LibrarySync, &server()).await;
        let other = config::Source::Server("srv-2".into());
        assert!(
            plan(&db, &other, Some(DAY))
                .await
                .due
                .contains(&JobKind::LibrarySync)
        );
    }

    fn config_every(minutes: u32) -> config::AppConfig {
        config::AppConfig {
            sync_interval_minutes: minutes,
            ..Default::default()
        }
    }

    fn minutes(count: u64) -> Duration {
        Duration::from_secs(count * 60)
    }

    /// Each check is counted and answered with the configured interval, as the real one does when nothing is stamped.
    fn counting() -> (
        Arc<AtomicUsize>,
        impl FnMut(config::AppConfig) -> std::future::Ready<Option<Duration>>,
    ) {
        let count = Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        let check = move |config: config::AppConfig| {
            seen.fetch_add(1, Ordering::SeqCst);
            std::future::ready(interval(&config))
        };
        (count, check)
    }

    async fn settle() {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }

    async fn pass(span: Duration) {
        tokio::time::advance(span).await;
        settle().await;
    }

    #[tokio::test(start_paused = true)]
    async fn the_interval_elapsing_runs_the_check_again_and_again() {
        let (_tx, rx) = watch::channel(config_every(60));
        let (count, check) = counting();
        tokio::spawn(drive(rx, check));
        settle().await;
        assert_eq!(count.load(Ordering::SeqCst), 1, "startup check");

        pass(minutes(59)).await;
        assert_eq!(count.load(Ordering::SeqCst), 1, "not yet");
        pass(minutes(2)).await;
        assert_eq!(count.load(Ordering::SeqCst), 2, "an hour on");
        pass(minutes(60)).await;
        assert_eq!(count.load(Ordering::SeqCst), 3, "re-armed after firing");
    }

    #[tokio::test(start_paused = true)]
    async fn changing_the_interval_re_arms_the_timer() {
        let (tx, rx) = watch::channel(config_every(60));
        let (count, check) = counting();
        tokio::spawn(drive(rx, check));
        settle().await;
        pass(minutes(10)).await;

        tx.send_modify(|config| config.sync_interval_minutes = 5);
        settle().await;
        assert_eq!(count.load(Ordering::SeqCst), 2, "rechecked on the change");

        pass(minutes(4)).await;
        assert_eq!(
            count.load(Ordering::SeqCst),
            2,
            "not before the new interval"
        );
        pass(minutes(2)).await;
        assert_eq!(
            count.load(Ordering::SeqCst),
            3,
            "the new interval, not the old hour"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_zero_interval_never_checks_again_until_it_is_raised() {
        let (tx, rx) = watch::channel(config_every(60));
        let (count, check) = counting();
        tokio::spawn(drive(rx, check));
        settle().await;

        tx.send_modify(|config| config.sync_interval_minutes = 0);
        settle().await;
        assert_eq!(count.load(Ordering::SeqCst), 2, "the change is seen once");
        pass(Duration::from_secs(7 * 24 * 60 * 60)).await;
        assert_eq!(count.load(Ordering::SeqCst), 2, "a week of silence");

        tx.send_modify(|config| config.sync_interval_minutes = 30);
        settle().await;
        assert_eq!(count.load(Ordering::SeqCst), 3, "turned back on");
        pass(minutes(31)).await;
        assert_eq!(count.load(Ordering::SeqCst), 4);
    }

    #[tokio::test(start_paused = true)]
    async fn starting_at_zero_checks_once_and_then_idles() {
        let (_tx, rx) = watch::channel(config_every(0));
        let (count, check) = counting();
        tokio::spawn(drive(rx, check));
        settle().await;
        pass(Duration::from_secs(7 * 24 * 60 * 60)).await;
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    /// Volume moves constantly; a timer reset by every config write would never fire.
    #[tokio::test(start_paused = true)]
    async fn an_unrelated_config_write_neither_rechecks_nor_delays_the_timer() {
        let (tx, rx) = watch::channel(config_every(60));
        let (count, check) = counting();
        tokio::spawn(drive(rx, check));
        settle().await;
        pass(minutes(30)).await;

        tx.send_modify(|config| config.volume = 0.3);
        settle().await;
        assert_eq!(count.load(Ordering::SeqCst), 1);
        pass(minutes(31)).await;
        assert_eq!(
            count.load(Ordering::SeqCst),
            2,
            "still on the original schedule"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_source_change_rechecks_at_once() {
        let (tx, rx) = watch::channel(config_every(60));
        let (count, check) = counting();
        tokio::spawn(drive(rx, check));
        settle().await;

        tx.send_modify(|config| config.active_source = server());
        settle().await;
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }
}
