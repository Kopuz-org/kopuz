//! Scans folder sources at startup, when one is created and when its folders change.

use std::path::PathBuf;
use std::sync::Arc;

use api::{ApiError, ApiEvent, ErrorCode, JobKind};
use tokio::sync::{broadcast, watch};

use crate::jobs::JobRunner;
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

/// Scan on startup and on every folder-source change, never off a config that failed to load.
pub fn spawn(
    jobs: Arc<JobRunner>,
    library: Arc<LibraryService>,
    config: watch::Receiver<config::AppConfig>,
    events: broadcast::Receiver<ApiEvent>,
    config_loaded: bool,
) {
    let start: StartScan = Arc::new(move || {
        library.spawn_scan(&jobs)?;
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
        let mut known = roots(&config.borrow_and_update());
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
                    let next = roots(&config.borrow_and_update());
                    if needs_scan(&known, &next) {
                        pending = true;
                        retry = true;
                    }
                    known = next;
                }
                event = events.recv() => match event {
                    Ok(ApiEvent::JobFinished { kind: JobKind::Scan, .. }) => retry = true,
                    // The lost events may have held the finish this was waiting on, so try again.
                    Err(broadcast::error::RecvError::Lagged(_)) => retry = true,
                    Err(broadcast::error::RecvError::Closed) => return,
                    _ => {}
                },
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

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
}
