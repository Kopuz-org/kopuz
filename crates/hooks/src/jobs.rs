//! Following a daemon job from the UI.
//!
//! A job's progress arrives on the event stream rather than being something
//! the caller counts itself, so a sync started by one frontend is visible in
//! every other one -- and in this one across a navigation, since the daemon
//! keeps running while the page that started it is gone.

use dioxus::prelude::*;

use crate::api::use_api;

/// What a job of one kind is doing right now.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct JobProgress {
    pub running: bool,
    /// Items handled so far, when the job counts them.
    pub current: Option<u64>,
    pub total: Option<u64>,
}

/// Follow every job of `kind`. Starts from the daemon's current job list, so
/// a page that mounts mid-sync shows the sync rather than nothing.
pub fn use_job_progress(kind: api::JobKind) -> Signal<JobProgress> {
    let api = use_api();
    let mut state = use_signal(JobProgress::default);

    use_future(move || {
        let api = api.clone();
        async move {
            if let Ok(jobs) = api.jobs().await
                && let Some(job) = jobs
                    .iter()
                    .find(|job| job.kind == kind && job.state == api::JobState::Running)
            {
                state.set(JobProgress {
                    running: true,
                    current: job.current,
                    total: job.total,
                });
            }

            let mut events = api.events();
            use futures_util::StreamExt;
            while let Some(event) = events.next().await {
                match event {
                    api::ApiEvent::JobProgress(progress) if progress.kind == kind => {
                        state.set(JobProgress {
                            running: true,
                            current: progress.current,
                            total: progress.total,
                        });
                    }
                    api::ApiEvent::JobFinished { kind: finished, .. } if finished == kind => {
                        state.set(JobProgress::default())
                    }
                    _ => {}
                }
            }
        }
    });

    state
}

/// Start a job, ignoring the conflict a second start of the same kind
/// returns -- one is already running, which is what the caller wanted.
pub fn start(kind: api::JobKind) {
    let api = crate::api::consume_api();
    spawn(async move {
        match api.start_job(kind).await {
            Ok(_) => {}
            Err(error) if error.code == api::ErrorCode::Conflict => {}
            Err(error) => {
                tracing::warn!(%error, ?kind, "could not start job");
                crate::toast::toast_error(&error.to_string());
            }
        }
    });
}
