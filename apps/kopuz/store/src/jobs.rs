use std::collections::{HashMap, HashSet};
use std::time::Duration;

use api::{ApiEvent, ErrorBody, ErrorCode, JobKind, JobProgress, JobState};
use gpui_kit::{App, Context, Entity, EventEmitter, SharedString, Subscription, Task};
use link::Link;

use crate::notices::{Notice, Notices};

/// How long a finished job stays on the activity list with its outcome.
pub const LINGER: Duration = Duration::from_secs(5);

/// A job reached its terminal state.
#[derive(Debug, Clone)]
pub struct JobEvent {
    pub id: String,
    pub kind: JobKind,
    pub ok: bool,
    pub error: Option<ErrorBody>,
    /// Started by the daemon's schedule rather than by anyone pressing anything.
    pub automatic: bool,
}

/// A job that has just stopped, kept on the list for [`LINGER`] so its outcome can be read.
#[derive(Debug, Clone)]
pub struct Finished {
    pub id: String,
    pub kind: JobKind,
    pub ok: bool,
    pub automatic: bool,
    pub message: Option<SharedString>,
}

/// The name a job kind answers to in `layout.notifications.jobs` and in `job_<kind>` strings.
pub fn kind_key(kind: JobKind) -> &'static str {
    match kind {
        JobKind::Scan => "scan",
        JobKind::LibrarySync => "library_sync",
        JobKind::FavoritesSync => "favorites_sync",
        JobKind::PlaylistSync => "playlist_sync",
        JobKind::Download => "download",
        JobKind::UrlDownload => "url_download",
        JobKind::AudioRip => "audio_rip",
        JobKind::Unknown => "job",
    }
}

/// What a job kind is called, for a line that names it.
pub fn kind_label(kind: JobKind) -> SharedString {
    SharedString::from(i18n::t(&format!("job_{}", kind_key(kind))))
}

/// Progress of the daemon's long-running work, by kind.
pub struct JobsModel {
    running: Vec<JobProgress>,
    /// Cancelled, but still on the daemon's list: the Stop chip reads *Stopping…* until it leaves.
    stopping: HashSet<String>,
    recent: Vec<Finished>,
    _linger: HashMap<String, Task<()>>,
    _fetch: Option<Task<()>>,
    _subscription: Subscription,
}

impl EventEmitter<JobEvent> for JobsModel {}

impl JobsModel {
    pub fn global(cx: &App) -> Entity<Self> {
        crate::handle(cx)
    }

    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let connection = Link::connection(cx);
        let subscription = cx.subscribe(&connection, |this, _, event: &ApiEvent, cx| match event {
            ApiEvent::JobProgress(progress) => {
                match this
                    .running
                    .iter_mut()
                    .find(|running| running.id == progress.id)
                {
                    Some(running) => *running = progress.clone(),
                    None => this.running.push(progress.clone()),
                }
                crate::notify(cx);
            }
            ApiEvent::JobFinished {
                id,
                kind,
                ok,
                error,
                automatic,
            } => {
                this.running.retain(|running| &running.id != id);
                this.stopping.remove(id);
                this.remember(
                    Finished {
                        id: id.clone(),
                        kind: *kind,
                        ok: *ok,
                        automatic: *automatic,
                        message: error.as_ref().map(crate::human_body),
                    },
                    cx,
                );
                crate::notify(cx);
                // A job that fails says so. Upstream drops this, which is why a sync against
                // expired credentials spins for a moment and then goes quiet.
                let notice = match (ok, error.as_ref()) {
                    (false, Some(failure)) => Some(Notice::new(
                        api::NoticeLevel::Error,
                        crate::human_body(failure).to_string(),
                    )),
                    (false, None) => Some(Notice::new(
                        api::NoticeLevel::Error,
                        i18n::t_with(
                            "job_failed",
                            &[("job", kind_label(*kind).to_string().into())],
                        ),
                    )),
                    // The schedule's own runs succeed on the activity list, not as a toast each interval.
                    (true, _) if *automatic => None,
                    (true, _) => Some(Notice::new(
                        api::NoticeLevel::Info,
                        i18n::t_with(
                            "job_finished",
                            &[("job", kind_label(*kind).to_string().into())],
                        ),
                    )),
                };
                if let Some(notice) = notice {
                    this.say(notice.kind(kind_key(*kind)), cx);
                }
                cx.emit(JobEvent {
                    id: id.clone(),
                    kind: *kind,
                    ok: *ok,
                    error: error.clone(),
                    automatic: *automatic,
                });
            }
            ApiEvent::Resync => this.reseed(cx),
            _ => {}
        });
        Self {
            running: Vec::new(),
            stopping: HashSet::new(),
            recent: Vec::new(),
            _linger: HashMap::new(),
            _fetch: None,
            _subscription: subscription,
        }
    }

    fn say(&self, notice: Notice, cx: &mut Context<Self>) {
        let notices = Notices::global(cx);
        notices.update(cx, |notices, cx| notices.say(notice, cx));
    }

    /// Keep a stopped job on the list briefly, so its outcome is readable where it was running.
    fn remember(&mut self, finished: Finished, cx: &mut Context<Self>) {
        let id = finished.id.clone();
        self.recent.retain(|held| held.id != id);
        self.recent.push(finished);
        let evicted = id.clone();
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(LINGER).await;
            this.update(cx, |this, cx| {
                this.recent.retain(|finished| finished.id != evicted);
                this._linger.remove(&evicted);
                crate::notify(cx);
            })
            .ok();
        });
        self._linger.insert(id, task);
    }

    fn reseed(&mut self, cx: &mut Context<Self>) {
        let call = Link::global(cx).call(|api| async move { api.jobs().await });
        self._fetch = Some(cx.spawn(async move |this, cx| {
            let Ok(jobs) = call.await else { return };
            this.update(cx, |this, cx| {
                this.running = jobs
                    .into_iter()
                    .filter(|job| job.state == JobState::Running)
                    .map(|job| JobProgress {
                        id: job.id,
                        kind: job.kind,
                        phase: job.phase,
                        current: job.current,
                        total: job.total,
                        message: job.message,
                        automatic: job.automatic,
                    })
                    .collect();
                this.stopping
                    .retain(|id| this.running.iter().any(|job| &job.id == id));
                crate::notify(cx);
            })
            .ok();
        }));
    }

    pub fn progress(&self, kind: JobKind) -> Option<&JobProgress> {
        self.running.iter().find(|job| job.kind == kind)
    }

    pub fn is_running(&self, kind: JobKind) -> bool {
        self.progress(kind).is_some()
    }

    pub fn running(&self) -> &[JobProgress] {
        &self.running
    }

    /// Jobs that stopped within the last [`LINGER`], newest last.
    pub fn recent(&self) -> &[Finished] {
        &self.recent
    }

    pub fn is_stopping(&self, id: &str) -> bool {
        self.stopping.contains(id)
    }

    /// Start a job; a kind already running answers `Conflict`, which is not an error here.
    pub fn start(&mut self, kind: JobKind, cx: &mut Context<Self>) {
        let call = Link::global(cx).call(move |api| async move { api.start_job(kind).await });
        cx.spawn(async move |_, cx| match call.await {
            Ok(_)
            | Err(api::ApiError {
                code: ErrorCode::Conflict,
                ..
            }) => {}
            Err(error) => {
                cx.update(|cx| Notices::failed(&error, cx));
            }
        })
        .detach();
    }

    /// Ask the daemon to stop a job. The chip says *Stopping…* until the job actually leaves.
    pub fn cancel(&mut self, id: String, cx: &mut Context<Self>) {
        if !self.stopping.insert(id.clone()) {
            return;
        }
        crate::notify(cx);
        let call = Link::global(cx).call({
            let id = id.clone();
            move |api| async move { api.cancel_job(id).await }
        });
        cx.spawn(async move |this, cx| {
            // A refused cancel used to vanish, leaving Stop looking like it had worked.
            if let Err(error) = call.await {
                this.update(cx, |this, cx| {
                    this.stopping.remove(&id);
                    crate::notify(cx);
                })
                .ok();
                cx.update(|cx| Notices::failed(&error, cx));
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_job_kind_has_a_name_to_configure_and_translate() {
        for kind in [
            JobKind::Scan,
            JobKind::LibrarySync,
            JobKind::FavoritesSync,
            JobKind::PlaylistSync,
            JobKind::Download,
            JobKind::UrlDownload,
            JobKind::Unknown,
        ] {
            assert!(!kind_key(kind).is_empty());
        }
        assert_eq!(kind_key(JobKind::Scan), "scan");
    }
}
