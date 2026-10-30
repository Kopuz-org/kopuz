//! Following a daemon job from the UI.
//!
//! A job's progress arrives on the event stream rather than being something
//! the caller counts itself, so a sync started by one frontend is visible in
//! every other one -- and in this one across a navigation, since the daemon
//! keeps running while the page that started it is gone.

use dioxus::prelude::*;

use crate::api::use_api;

/// What a job of one kind is doing right now.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct JobProgress {
    pub running: bool,
    /// What the job is doing now: "starting", then whatever the job names its
    /// stages -- a page picks its icon from this.
    pub phase: String,
    /// Items handled so far, when the job counts them.
    pub current: Option<u64>,
    pub total: Option<u64>,
    /// What the job is working on -- the download job names the track it
    /// is fetching, which is how a row knows it is the active one.
    pub message: Option<String>,
}

fn running_job(jobs: &[api::JobStatus], kind: api::JobKind) -> JobProgress {
    jobs.iter()
        .find(|job| job.kind == kind && job.state == api::JobState::Running)
        .map(|job| JobProgress {
            running: true,
            phase: job.phase.clone(),
            current: job.current,
            message: job.message.clone(),
            total: job.total,
        })
        .unwrap_or_default()
}

/// Follow every job of `kind`. Starts from the daemon's current job list, so
/// a page that mounts mid-sync shows the sync rather than nothing.
pub fn use_job_progress(kind: api::JobKind) -> Signal<JobProgress> {
    let api = use_api();
    let mut state = use_signal(JobProgress::default);

    use_future(move || {
        let api = api.clone();
        async move {
            let mut events = api.events();
            match api.jobs().await {
                Ok(jobs) => state.set(running_job(&jobs, kind)),
                Err(error) => tracing::warn!(%error, "could not load job status"),
            }
            use futures_util::StreamExt;
            while let Some(event) = events.next().await {
                match event {
                    api::ApiEvent::Resync => match api.jobs().await {
                        Ok(jobs) => state.set(running_job(&jobs, kind)),
                        Err(error) => tracing::warn!(%error, "could not reload job status"),
                    },
                    api::ApiEvent::JobProgress(progress) if progress.kind == kind => {
                        state.set(JobProgress {
                            running: true,
                            phase: progress.phase.clone(),
                            current: progress.current,
                            message: progress.message.clone(),
                            total: progress.total,
                        });
                    }
                    api::ApiEvent::JobFinished { kind: finished, .. } if finished == kind => {
                        state.set(JobProgress::default())
                    }
                    _ => {}
                }
            }
            state.set(JobProgress::default());
        }
    });

    state
}

/// Whether a scan or sync that fills the library is running, so an empty listing is not the last word yet.
pub fn use_library_filling() -> Memo<bool> {
    let scan = use_job_progress(api::JobKind::Scan);
    let library = use_job_progress(api::JobKind::LibrarySync);
    let playlists = use_job_progress(api::JobKind::PlaylistSync);
    let favorites = use_job_progress(api::JobKind::FavoritesSync);
    use_memo(move || {
        scan.read().running
            || library.read().running
            || playlists.read().running
            || favorites.read().running
    })
}

/// Toast a finished job's failure; the app root calls it once per event, so a failure is said once however many pages follow the job.
pub fn say_failure(error: Option<&api::ErrorBody>) {
    if let Some(error) = error {
        crate::toast::toast_api_error(&error.clone().into());
    }
}

/// Start a job; while one of `kind` runs, the daemon joins it instead.
pub fn start(kind: api::JobKind) {
    let api = crate::api::consume_api();
    spawn(async move {
        if let Err(error) = api.start_job(kind).await {
            tracing::warn!(%error, ?kind, "could not start job");
            crate::toast::toast_api_error(&error);
        }
    });
}
