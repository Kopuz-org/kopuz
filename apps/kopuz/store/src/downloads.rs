use std::collections::HashMap;
use std::collections::HashSet;

use api::{DownloadItemState, DownloadItemStatus, JobKind, Table};
use gpui_kit::{App, Context, Entity, EventEmitter, SharedString, Subscription, Task};
use link::Link;

use crate::invalidation::{Invalidation, TablesChanged};
use crate::jobs::{JobEvent, JobsModel};
use crate::notices::Notices;

/// Where one row of the download queue has got to; `Done` is this client's, since the daemon forgets a finished item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemState {
    Queued,
    Downloading,
    Failed,
    Done,
}

impl From<DownloadItemState> for ItemState {
    fn from(state: DownloadItemState) -> Self {
        match state {
            DownloadItemState::Queued => ItemState::Queued,
            DownloadItemState::Downloading => ItemState::Downloading,
            DownloadItemState::Failed => ItemState::Failed,
        }
    }
}

/// One requested download, named the way a row shows it.
#[derive(Debug, Clone)]
pub struct DownloadItem {
    pub key: String,
    pub title: SharedString,
    pub artist: SharedString,
    pub state: ItemState,
}

/// A batch reached its end; the chrome turns it into the summary toast.
#[derive(Debug, Clone, Copy)]
pub struct BatchFinished {
    pub downloaded: usize,
    pub failed: usize,
}

/// What a set of keys says about offline copies: every one stored, any one still on its way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Offline {
    pub stored: bool,
    pub busy: bool,
}

/// Which tracks have an offline copy, and what the running download batch is doing.
pub struct DownloadsModel {
    stored: HashSet<String>,
    active: HashMap<String, DownloadItemState>,
    /// The batch as rows, kept past the end of the job so the failures stay reachable.
    queue: Vec<DownloadItem>,
    /// The job the rows belong to, so the next batch starts a fresh list.
    job: Option<String>,
    batch: Option<BatchFinished>,
    _fetch: Option<Task<()>>,
    _statuses: Option<Task<()>>,
    _titles: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<BatchFinished> for DownloadsModel {}

impl DownloadsModel {
    pub fn global(cx: &App) -> Entity<Self> {
        crate::handle(cx)
    }

    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let invalidation = Invalidation::global(cx);
        let jobs = JobsModel::global(cx);
        let subscriptions = vec![
            cx.subscribe(&invalidation, |this, _, event: &TablesChanged, cx| {
                if event.touches(&[Table::Tracks]) {
                    this.refetch(cx);
                }
            }),
            cx.observe(&jobs, |this, jobs, cx| {
                let Some(id) = jobs
                    .read(cx)
                    .progress(JobKind::Download)
                    .map(|job| job.id.clone())
                else {
                    return;
                };
                // A new job is a new batch: the rows of the last one stop being the answer.
                if this.job.as_ref() != Some(&id) {
                    this.job = Some(id);
                    this.queue.clear();
                    this.batch = None;
                }
                this.refresh_statuses(cx);
            }),
            cx.subscribe(&jobs, |this, _, event: &JobEvent, cx| {
                if event.kind == JobKind::Download {
                    this.active.clear();
                    this.job = None;
                    this.finish_batch(cx);
                    this.refetch(cx);
                }
            }),
        ];
        Self {
            stored: HashSet::new(),
            active: HashMap::new(),
            queue: Vec::new(),
            job: None,
            batch: None,
            _fetch: None,
            _statuses: None,
            _titles: None,
            _subscriptions: subscriptions,
        }
    }

    fn refetch(&mut self, cx: &mut Context<Self>) {
        let call = Link::global(cx).call(|api| async move { api.downloads().await });
        self._fetch = Some(cx.spawn(async move |this, cx| {
            let Ok(keys) = call.await else { return };
            this.update(cx, |this, cx| {
                this.stored = keys.into_iter().collect();
                crate::notify(cx);
            })
            .ok();
        }));
    }

    fn refresh_statuses(&mut self, cx: &mut Context<Self>) {
        if self._statuses.is_some() {
            return;
        }
        let call = Link::global(cx).call(|api| async move { api.download_statuses().await });
        self._statuses = Some(cx.spawn(async move |this, cx| {
            let statuses = call.await;
            this.update(cx, |this, cx| {
                this._statuses = None;
                if let Ok(statuses) = statuses {
                    this.active = statuses
                        .iter()
                        .map(|status| (status.key.clone(), status.state))
                        .collect();
                    this.sync_queue(&statuses, cx);
                    crate::notify(cx);
                }
            })
            .ok();
        }));
    }

    /// The batch reports keys; a row wants a title, so the library is asked once per new key.
    fn sync_queue(&mut self, statuses: &[DownloadItemStatus], cx: &mut Context<Self>) {
        // A key the daemon has stopped reporting is done, not gone: the row says so and stays.
        for item in self.queue.iter_mut() {
            let reported = statuses.iter().any(|status| status.key == item.key);
            if !reported && item.state != ItemState::Failed {
                item.state = ItemState::Done;
            }
        }
        for status in statuses {
            match self.queue.iter_mut().find(|item| item.key == status.key) {
                Some(item) => item.state = status.state.into(),
                None => self.queue.push(DownloadItem {
                    key: status.key.clone(),
                    title: SharedString::from(status.key.clone()),
                    artist: SharedString::default(),
                    state: status.state.into(),
                }),
            }
        }
        self.name_rows(cx);
    }

    fn name_rows(&mut self, cx: &mut Context<Self>) {
        if self._titles.is_some() {
            return;
        }
        let unnamed: Vec<String> = self
            .queue
            .iter()
            .filter(|item| item.title == item.key.as_str())
            .map(|item| item.key.clone())
            .collect();
        if unnamed.is_empty() {
            return;
        }
        let call =
            Link::global(cx).call(move |api| async move { api.tracks_by_keys(unnamed).await });
        self._titles = Some(cx.spawn(async move |this, cx| {
            let tracks = call.await;
            this.update(cx, |this, cx| {
                this._titles = None;
                let Ok(tracks) = tracks else { return };
                for track in tracks {
                    let Some(item) = this.queue.iter_mut().find(|item| item.key == track.key)
                    else {
                        continue;
                    };
                    if !track.title.is_empty() {
                        item.title = SharedString::from(track.title);
                    }
                    item.artist = SharedString::from(track.artist);
                }
                crate::notify(cx);
            })
            .ok();
        }));
    }

    /// The job stopped: whatever had not failed is done, and the count is worth saying once.
    fn finish_batch(&mut self, cx: &mut Context<Self>) {
        if self.queue.is_empty() {
            return;
        }
        for item in self.queue.iter_mut() {
            if item.state != ItemState::Failed {
                item.state = ItemState::Done;
            }
        }
        let failed = self
            .queue
            .iter()
            .filter(|item| item.state == ItemState::Failed)
            .count();
        let summary = BatchFinished {
            downloaded: self.queue.len() - failed,
            failed,
        };
        self.batch = Some(summary);
        crate::notify(cx);
        cx.emit(summary);
    }

    pub fn is_stored(&self, key: &str) -> bool {
        self.stored.contains(key)
    }

    pub fn state(&self, key: &str) -> Option<DownloadItemState> {
        self.active.get(key).copied()
    }

    pub fn is_active(&self, key: &str) -> bool {
        matches!(
            self.state(key),
            Some(DownloadItemState::Queued | DownloadItemState::Downloading)
        )
    }

    /// The one stored/busy answer for a key set; compute it on a data change, not per frame.
    pub fn offline(&self, keys: &[String]) -> Offline {
        Offline {
            stored: !keys.is_empty() && keys.iter().all(|key| self.is_stored(key)),
            busy: !self.active.is_empty() && keys.iter().any(|key| self.is_active(key)),
        }
    }

    /// Download the keys, or give their offline copies back.
    pub fn set_offline(&mut self, keys: Vec<String>, offline: bool, cx: &mut Context<Self>) {
        if offline {
            self.download(keys, cx);
            return;
        }
        for key in keys {
            self.remove(key, cx);
        }
    }

    pub fn stored(&self) -> &HashSet<String> {
        &self.stored
    }

    pub fn active(&self) -> &HashMap<String, DownloadItemState> {
        &self.active
    }

    /// The batch as rows: what is queued, what is going, what failed, what is done.
    pub fn queue(&self) -> &[DownloadItem] {
        &self.queue
    }

    /// The outcome of the last batch, until a new one starts.
    pub fn batch(&self) -> Option<BatchFinished> {
        self.batch
    }

    pub fn failed_keys(&self) -> Vec<String> {
        self.queue
            .iter()
            .filter(|item| item.state == ItemState::Failed)
            .map(|item| item.key.clone())
            .collect()
    }

    /// Put the failures back on the daemon's queue, as their own batch.
    pub fn retry_failed(&mut self, cx: &mut Context<Self>) {
        let keys = self.failed_keys();
        if keys.is_empty() {
            return;
        }
        for item in self.queue.iter_mut() {
            if item.state == ItemState::Failed {
                item.state = ItemState::Queued;
            }
        }
        self.batch = None;
        crate::notify(cx);
        self.download(keys, cx);
    }

    /// Forget the rows of a batch that is no longer running.
    pub fn clear_queue(&mut self, cx: &mut Context<Self>) {
        if self.queue.is_empty() && self.batch.is_none() {
            return;
        }
        self.queue.clear();
        self.batch = None;
        crate::notify(cx);
    }

    pub fn download(&mut self, keys: Vec<String>, cx: &mut Context<Self>) {
        let call = Link::global(cx).call(move |api| async move { api.download(keys).await });
        cx.spawn(async move |_, cx| {
            if let Err(error) = call.await {
                cx.update(|cx| Notices::failed(&error, cx));
            }
        })
        .detach();
    }

    pub fn remove(&mut self, key: String, cx: &mut Context<Self>) {
        self.stored.remove(&key);
        crate::notify(cx);
        let call = Link::global(cx).call(move |api| async move { api.remove_download(key).await });
        cx.spawn(async move |this, cx| {
            if let Err(error) = call.await {
                cx.update(|cx| Notices::failed(&error, cx));
            }
            this.update(cx, |this, cx| this.refetch(cx)).ok();
        })
        .detach();
    }
}
