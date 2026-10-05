//! Scans folder sources at startup, when one is created, when its folders change and once per sync interval.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use api::{ApiError, ApiEvent, ErrorCode, JobKind};
use tokio::sync::{broadcast, watch};
use tokio::time::Instant;

use crate::auto_sync::{interval, until};
use crate::jobs::{JobRunner, Trigger};
use crate::library::LibraryService;

type Roots = Vec<(String, Vec<PathBuf>)>;

/// Starts one scan job; `Conflict` means one is already running.
pub type StartScan = Arc<dyn Fn() -> Result<(), ApiError> + Send + Sync>;

fn roots(config: &config::AppConfig) -> Roots {
    config
        .local_sources
        .iter()
        .map(|source| (source.id.clone(), source.directories.clone()))
        .collect()
}

/// A root that is new or whose folders changed; a pure removal needs no scan.
fn needs_scan(previous: &Roots, next: &Roots) -> bool {
    next.iter().any(|root| !previous.contains(root))
}

/// When the next periodic rescan falls: one interval after the last scan, if there is anything to scan.
fn next_rescan(known: &Roots, every: Option<Duration>, last: Instant) -> Option<Instant> {
    if known.is_empty() {
        return None;
    }
    every.map(|every| last + every)
}

/// Scan on startup, on every folder-source change and once per sync interval, never off a config that failed to load.
pub fn spawn(
    jobs: Arc<JobRunner>,
    library: Arc<LibraryService>,
    config: watch::Receiver<config::AppConfig>,
    events: broadcast::Receiver<ApiEvent>,
    config_loaded: bool,
) {
    let start: StartScan = Arc::new(move || {
        library.spawn_scan(&jobs, Trigger::Schedule)?;
        Ok(())
    });
    spawn_with(start, config, events, config_loaded);
}

pub(crate) fn spawn_with(
    start: StartScan,
    mut config: watch::Receiver<config::AppConfig>,
    mut events: broadcast::Receiver<ApiEvent>,
    config_loaded: bool,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if !config_loaded {
            tracing::warn!("settings did not load; folder sources are not scanned this session");
            return;
        }
        let (mut known, mut every) = {
            let held = config.borrow_and_update();
            (roots(&held), interval(&held))
        };
        // The startup scan below counts as the last one, so the first rescan is a full interval out.
        let mut last = Instant::now();
        let mut pending = !known.is_empty();
        let mut retry = pending;
        loop {
            if pending && retry {
                retry = false;
                match start() {
                    Ok(()) => pending = false,
                    Err(error) if error.code == ErrorCode::Conflict => {}
                    Err(error) => {
                        tracing::warn!(%error, "folder scan could not start");
                        pending = false;
                    }
                }
            }
            tokio::select! {
                changed = config.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    let (next, next_every) = {
                        let held = config.borrow_and_update();
                        (roots(&held), interval(&held))
                    };
                    if needs_scan(&known, &next) {
                        pending = true;
                        retry = true;
                    }
                    known = next;
                    every = next_every;
                }
                event = events.recv() => match event {
                    Ok(ApiEvent::JobFinished { kind: JobKind::Scan, .. }) => {
                        retry = true;
                        last = Instant::now();
                    }
                    // The lost events may have held the finish this was waiting on, so try again.
                    Err(broadcast::error::RecvError::Lagged(_)) => retry = true,
                    Err(broadcast::error::RecvError::Closed) => return,
                    _ => {}
                },
                () = until(next_rescan(&known, every, last)) => {
                    last = Instant::now();
                    match start() {
                        Ok(()) => {}
                        // The scan in flight is the rescan; its finish moves the clock.
                        Err(error) if error.code == ErrorCode::Conflict => {}
                        Err(error) => tracing::warn!(%error, "periodic folder rescan could not start"),
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn config_with(dirs: &[&str]) -> config::AppConfig {
        let mut config = config::AppConfig::default();
        config.local_sources = vec![config::SavedLocalSource::default_library(
            dirs.iter().map(PathBuf::from).collect(),
        )];
        config
    }

    fn counter() -> (Arc<AtomicUsize>, StartScan) {
        let count = Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        let start: StartScan = Arc::new(move || {
            seen.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
        (count, start)
    }

    async fn settle() {
        tokio::time::sleep(Duration::from_millis(80)).await;
    }

    #[tokio::test]
    async fn changing_a_folder_list_schedules_a_scan() {
        let (count, start) = counter();
        let (tx, rx) = watch::channel(config_with(&["/music"]));
        let (events, events_rx) = broadcast::channel(8);
        let _keep = events;
        spawn_with(start, rx, events_rx, true);
        settle().await;
        assert_eq!(count.load(Ordering::SeqCst), 1, "startup scan");

        tx.send_modify(|config| config.volume = 0.5);
        settle().await;
        assert_eq!(count.load(Ordering::SeqCst), 1, "unrelated change");

        tx.send_replace(config_with(&["/music", "/more"]));
        settle().await;
        assert_eq!(count.load(Ordering::SeqCst), 2, "folder list changed");

        tx.send_modify(|config| config.local_sources.clear());
        settle().await;
        assert_eq!(count.load(Ordering::SeqCst), 2, "removal needs no scan");
    }

    #[tokio::test]
    async fn a_busy_runner_retries_when_the_running_scan_finishes() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let seen = attempts.clone();
        let start: StartScan = Arc::new(move || {
            if seen.fetch_add(1, Ordering::SeqCst) == 0 {
                Err(ApiError::new(ErrorCode::Conflict, "busy"))
            } else {
                Ok(())
            }
        });
        let (_tx, rx) = watch::channel(config_with(&["/music"]));
        let (events, events_rx) = broadcast::channel(8);
        spawn_with(start, rx, events_rx, true);
        settle().await;
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        events
            .send(ApiEvent::JobFinished {
                id: "job-1".into(),
                kind: JobKind::Scan,
                ok: true,
                error: None,
                automatic: false,
            })
            .expect("receiver alive");
        settle().await;
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_failed_config_load_never_schedules_a_scan() {
        let (count, start) = counter();
        let (tx, rx) = watch::channel(config_with(&["/music"]));
        let (events, events_rx) = broadcast::channel(8);
        let _keep = events;
        spawn_with(start, rx, events_rx, false);
        tx.send_replace(config_with(&["/elsewhere"]));
        settle().await;
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    fn every(minutes: u32, dirs: &[&str]) -> config::AppConfig {
        let mut config = config_with(dirs);
        config.sync_interval_minutes = minutes;
        config
    }

    fn minutes(count: u64) -> Duration {
        Duration::from_secs(count * 60)
    }

    async fn pass(span: Duration) {
        tokio::time::advance(span).await;
        tokio::time::sleep(Duration::from_millis(1)).await;
    }

    fn scan_finished() -> ApiEvent {
        ApiEvent::JobFinished {
            id: "job-1".into(),
            kind: JobKind::Scan,
            ok: true,
            error: None,
            automatic: false,
        }
    }

    /// The startup scan is the first one of the interval, so boot never scans twice.
    #[tokio::test(start_paused = true)]
    async fn folders_are_rescanned_once_per_interval_after_the_startup_scan() {
        let (count, start) = counter();
        let (_tx, rx) = watch::channel(every(60, &["/music"]));
        let (events, events_rx) = broadcast::channel(8);
        let _keep = events;
        spawn_with(start, rx, events_rx, true);
        pass(Duration::from_millis(1)).await;
        assert_eq!(count.load(Ordering::SeqCst), 1, "startup scan only");

        pass(minutes(59)).await;
        assert_eq!(count.load(Ordering::SeqCst), 1, "not yet");
        pass(minutes(2)).await;
        assert_eq!(count.load(Ordering::SeqCst), 2, "an hour after startup");
        pass(minutes(60)).await;
        assert_eq!(count.load(Ordering::SeqCst), 3, "and again");
    }

    /// A scan someone else ran counts, so the schedule never stacks one on top of it.
    #[tokio::test(start_paused = true)]
    async fn a_finished_scan_pushes_the_next_rescan_back() {
        let (count, start) = counter();
        let (_tx, rx) = watch::channel(every(60, &["/music"]));
        let (events, events_rx) = broadcast::channel(8);
        spawn_with(start, rx, events_rx, true);
        pass(Duration::from_millis(1)).await;
        pass(minutes(50)).await;
        events.send(scan_finished()).expect("receiver alive");
        pass(Duration::from_millis(1)).await;
        pass(minutes(50)).await;
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "100 minutes on, 50 since a scan"
        );
        pass(minutes(11)).await;
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_zero_interval_never_rescans_until_it_is_raised() {
        let (count, start) = counter();
        let (tx, rx) = watch::channel(every(0, &["/music"]));
        let (events, events_rx) = broadcast::channel(8);
        let _keep = events;
        spawn_with(start, rx, events_rx, true);
        pass(Duration::from_millis(1)).await;
        pass(Duration::from_secs(7 * 24 * 60 * 60)).await;
        assert_eq!(count.load(Ordering::SeqCst), 1, "startup scan only");

        tx.send_modify(|config| config.sync_interval_minutes = 30);
        pass(Duration::from_millis(1)).await;
        assert_eq!(
            count.load(Ordering::SeqCst),
            2,
            "a week stale is due at once"
        );
        pass(minutes(31)).await;
        assert_eq!(count.load(Ordering::SeqCst), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn changing_the_interval_re_arms_the_rescan() {
        let (count, start) = counter();
        let (tx, rx) = watch::channel(every(120, &["/music"]));
        let (events, events_rx) = broadcast::channel(8);
        let _keep = events;
        spawn_with(start, rx, events_rx, true);
        pass(Duration::from_millis(1)).await;
        pass(minutes(10)).await;

        tx.send_modify(|config| config.sync_interval_minutes = 60);
        pass(Duration::from_millis(1)).await;
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "10 minutes in, an hour is not up"
        );
        pass(minutes(51)).await;
        assert_eq!(
            count.load(Ordering::SeqCst),
            2,
            "an hour after startup, not two"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_unrelated_config_write_does_not_delay_the_rescan() {
        let (count, start) = counter();
        let (tx, rx) = watch::channel(every(60, &["/music"]));
        let (events, events_rx) = broadcast::channel(8);
        let _keep = events;
        spawn_with(start, rx, events_rx, true);
        pass(Duration::from_millis(1)).await;
        pass(minutes(30)).await;
        tx.send_modify(|config| config.volume = 0.2);
        pass(minutes(31)).await;
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn with_no_folders_there_is_nothing_to_rescan() {
        let (count, start) = counter();
        let mut config = every(60, &[]);
        config.local_sources.clear();
        let (_tx, rx) = watch::channel(config);
        let (events, events_rx) = broadcast::channel(8);
        let _keep = events;
        spawn_with(start, rx, events_rx, true);
        pass(minutes(180)).await;
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    /// A rescan that finds the runner busy does not queue another behind the running one.
    #[tokio::test(start_paused = true)]
    async fn a_busy_runner_at_the_rescan_does_not_queue_a_second_scan() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let seen = attempts.clone();
        let start: StartScan = Arc::new(move || {
            if seen.fetch_add(1, Ordering::SeqCst) == 1 {
                Err(ApiError::new(ErrorCode::Conflict, "busy"))
            } else {
                Ok(())
            }
        });
        let (_tx, rx) = watch::channel(every(60, &["/music"]));
        let (events, events_rx) = broadcast::channel(8);
        spawn_with(start, rx, events_rx, true);
        pass(Duration::from_millis(1)).await;
        pass(minutes(61)).await;
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            2,
            "startup, then the busy rescan"
        );
        events.send(scan_finished()).expect("receiver alive");
        pass(Duration::from_millis(1)).await;
        assert_eq!(attempts.load(Ordering::SeqCst), 2, "no retry of a rescan");
    }
}
