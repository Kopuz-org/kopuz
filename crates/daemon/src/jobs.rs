//! The job runner: long-running library work (scan, sync, downloads) as
//! addressable jobs with progress events, single-flight per kind, and
//! cooperative cancellation.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use api::{ApiError, ApiEvent, ErrorCode, JobKind, JobRef, JobState, JobStatus};

use crate::session::SessionHandle;

const RETAINED_JOBS: usize = 50;
const PROGRESS_THROTTLE_MS: u128 = 200;

struct JobEntry {
    status: JobStatus,
    cancelled: Arc<AtomicBool>,
}

pub struct JobRunner {
    session: SessionHandle,
    next_id: AtomicU64,
    entries: Arc<Mutex<Vec<JobEntry>>>,
}

/// Handed to a running job: progress reporting (throttled into `job.progress`
/// events) and the cooperative cancellation flag, checked between chunks.
#[derive(Clone)]
pub struct JobCtx {
    id: String,
    kind: JobKind,
    cancelled: Arc<AtomicBool>,
    session: SessionHandle,
    entries: Arc<Mutex<Vec<JobEntry>>>,
    last_emit: Arc<Mutex<Instant>>,
}

impl JobCtx {
    pub fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    pub fn progress(
        &self,
        phase: &str,
        current: Option<u64>,
        total: Option<u64>,
        message: Option<String>,
    ) {
        self.record(phase, current, total, message.clone());
        self.session
            .emit_event(ApiEvent::JobProgress(api::JobProgress {
                id: self.id.clone(),
                kind: self.kind,
                phase: phase.to_string(),
                current,
                total,
                message,
            }));
    }

    /// Progress capped at ~5 events/s, for per-file callbacks that would
    /// otherwise flood the event ring.
    pub fn progress_throttled(&self, phase: &str, message: Option<String>) {
        let due = {
            let Ok(mut last) = self.last_emit.lock() else {
                return;
            };
            if last.elapsed().as_millis() < PROGRESS_THROTTLE_MS {
                false
            } else {
                *last = Instant::now();
                true
            }
        };
        if due {
            self.progress(phase, None, None, message);
        }
    }

    fn record(
        &self,
        phase: &str,
        current: Option<u64>,
        total: Option<u64>,
        message: Option<String>,
    ) {
        if let Ok(mut entries) = self.entries.lock()
            && let Some(entry) = entries.iter_mut().find(|entry| entry.status.id == self.id)
        {
            entry.status.phase = phase.to_string();
            entry.status.current = current;
            entry.status.total = total;
            entry.status.message = message;
        }
    }
}

impl JobRunner {
    pub fn new(session: SessionHandle) -> Self {
        Self {
            session,
            next_id: AtomicU64::new(0),
            entries: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn start<F, Fut>(&self, kind: JobKind, job: F) -> Result<JobRef, ApiError>
    where
        F: FnOnce(JobCtx) -> Fut,
        Fut: std::future::Future<Output = Result<(), ApiError>> + Send + 'static,
    {
        let id = format!("job-{}", self.next_id.fetch_add(1, Ordering::Relaxed) + 1);
        let cancelled = Arc::new(AtomicBool::new(false));
        {
            let Ok(mut entries) = self.entries.lock() else {
                return Err(ApiError::internal("job registry poisoned"));
            };
            if entries
                .iter()
                .any(|entry| entry.status.kind == kind && entry.status.state == JobState::Running)
            {
                return Err(ApiError::new(
                    ErrorCode::Conflict,
                    "a job of this kind is already running",
                ));
            }
            if entries.len() >= RETAINED_JOBS {
                let drop_at = entries
                    .iter()
                    .position(|entry| entry.status.state != JobState::Running);
                if let Some(drop_at) = drop_at {
                    entries.remove(drop_at);
                }
            }
            entries.push(JobEntry {
                status: JobStatus {
                    id: id.clone(),
                    kind,
                    state: JobState::Running,
                    phase: "starting".to_string(),
                    current: None,
                    total: None,
                    message: None,
                    error: None,
                },
                cancelled: cancelled.clone(),
            });
        }

        let ctx = JobCtx {
            id: id.clone(),
            kind,
            cancelled: cancelled.clone(),
            session: self.session.clone(),
            entries: self.entries.clone(),
            last_emit: Arc::new(Mutex::new(Instant::now())),
        };
        let entries = self.entries.clone();
        let session = self.session.clone();
        let job_id = id.clone();
        let future = job(ctx);
        tokio::spawn(async move {
            let result = future.await;
            let (state, error) = if cancelled.load(Ordering::Relaxed) {
                (JobState::Cancelled, None)
            } else {
                match result {
                    Ok(()) => (JobState::Finished, None),
                    Err(error) => (JobState::Failed, Some(error.body())),
                }
            };
            if let Ok(mut entries) = entries.lock()
                && let Some(entry) = entries.iter_mut().find(|entry| entry.status.id == job_id)
            {
                entry.status.state = state;
                entry.status.error = error.clone();
            }
            session.emit_event(ApiEvent::JobFinished {
                id: job_id,
                kind,
                ok: state == JobState::Finished,
                error,
            });
        });
        Ok(JobRef { job_id: id })
    }

    pub fn list(&self) -> Vec<JobStatus> {
        self.entries
            .lock()
            .map(|entries| entries.iter().map(|entry| entry.status.clone()).collect())
            .unwrap_or_default()
    }

    pub fn cancel(&self, id: &str) -> Result<(), ApiError> {
        let Ok(entries) = self.entries.lock() else {
            return Err(ApiError::internal("job registry poisoned"));
        };
        let Some(entry) = entries.iter().find(|entry| entry.status.id == id) else {
            return Err(ApiError::not_found("no such job"));
        };
        if entry.status.state != JobState::Running {
            return Err(ApiError::invalid_input("job is not running"));
        }
        entry.cancelled.store(true, Ordering::Relaxed);
        Ok(())
    }
}
