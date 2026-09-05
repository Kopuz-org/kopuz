use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use api::{DownloadItemState, KopuzApi};
use dioxus::core::spawn_forever;
use dioxus::prelude::*;

#[derive(Clone, Copy)]
pub struct DownloadedTracks(pub Signal<HashSet<String>>);

pub fn is_downloaded(key: &str) -> bool {
    consume_context::<DownloadedTracks>().0.read().contains(key)
}

pub fn downloaded_keys() -> HashSet<String> {
    consume_context::<DownloadedTracks>().0.read().clone()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadStatus {
    Queued,
    Downloading,
    Done,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadItem {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub status: DownloadStatus,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

#[derive(Clone, Debug, Default)]
pub struct DownloadQueue {
    pub items: Vec<DownloadItem>,
    pub bytes_done_session: u64,
    pub session_elapsed_secs: f64,
    driver_running: bool,
    active_job_id: Option<String>,
    started_at: Option<Instant>,
}

impl DownloadQueue {
    pub fn is_active(&self) -> bool {
        self.items.iter().any(|item| {
            matches!(
                item.status,
                DownloadStatus::Queued | DownloadStatus::Downloading
            )
        })
    }

    pub fn done_count(&self) -> usize {
        self.items
            .iter()
            .filter(|item| matches!(item.status, DownloadStatus::Done))
            .count()
    }

    pub fn total_non_failed(&self) -> usize {
        self.items
            .iter()
            .filter(|item| {
                !matches!(
                    item.status,
                    DownloadStatus::Failed | DownloadStatus::Cancelled
                )
            })
            .count()
    }

    pub fn dismiss(&mut self) {
        self.items.retain(|item| {
            matches!(
                item.status,
                DownloadStatus::Queued | DownloadStatus::Downloading
            )
        });
        if self.items.is_empty() {
            self.bytes_done_session = 0;
            self.session_elapsed_secs = 0.0;
        }
    }
}

fn apply_statuses(queue: &mut DownloadQueue, statuses: &[api::DownloadItemStatus]) {
    for status in statuses {
        let Some(item) = queue.items.iter_mut().find(|item| item.id == status.key) else {
            continue;
        };
        item.status = match status.state {
            DownloadItemState::Queued | DownloadItemState::Unknown => DownloadStatus::Queued,
            DownloadItemState::Downloading => DownloadStatus::Downloading,
            DownloadItemState::Finished => DownloadStatus::Done,
            DownloadItemState::Failed => DownloadStatus::Failed,
            DownloadItemState::Cancelled => DownloadStatus::Cancelled,
        };
        item.bytes_done = status.bytes_done;
        item.bytes_total = status.total_bytes.unwrap_or_default();
    }
    queue.bytes_done_session = queue.items.iter().map(|item| item.bytes_done).sum();
    queue.session_elapsed_secs = queue
        .started_at
        .map(|started| started.elapsed().as_secs_f64())
        .unwrap_or_default();
}

async fn drive_download_queue(
    api: Arc<dyn KopuzApi>,
    mut queue: Signal<DownloadQueue>,
    mut downloaded: DownloadedTracks,
) {
    loop {
        let queued: Vec<String> = queue
            .peek()
            .items
            .iter()
            .filter(|item| matches!(item.status, DownloadStatus::Queued))
            .map(|item| item.id.clone())
            .collect();
        if queued.is_empty() {
            break;
        }

        // A failed read must not clobber the shared downloaded set (offline
        // badges everywhere) or forget which keys are already on disk; fall
        // back to the last known set instead.
        let cached: HashSet<String> = match api.downloads().await {
            Ok(keys) => {
                let cached: HashSet<String> = keys.into_iter().collect();
                downloaded.0.set(cached.clone());
                cached
            }
            Err(error) => {
                tracing::warn!(%error, "could not list daemon downloads");
                downloaded.0.peek().clone()
            }
        };
        {
            let mut state = queue.write();
            for item in &mut state.items {
                if queued.contains(&item.id) && cached.contains(&item.id) {
                    item.status = DownloadStatus::Done;
                }
            }
        }
        let pending: Vec<String> = queued
            .into_iter()
            .filter(|key| !cached.contains(key))
            .collect();
        if pending.is_empty() {
            continue;
        }

        let job = match api.download(pending.clone()).await {
            Ok(job) => job,
            Err(error) => {
                tracing::warn!(%error, "could not start daemon download job");
                let mut state = queue.write();
                for item in &mut state.items {
                    if pending.contains(&item.id) {
                        item.status = DownloadStatus::Failed;
                    }
                }
                break;
            }
        };
        {
            let mut state = queue.write();
            state.active_job_id = Some(job.job_id);
            state.started_at.get_or_insert_with(Instant::now);
        }

        loop {
            match api.download_statuses().await {
                Ok(statuses) => {
                    apply_statuses(&mut queue.write(), &statuses);
                    downloaded.0.write().extend(
                        statuses
                            .iter()
                            .filter(|status| status.state == DownloadItemState::Finished)
                            .map(|status| status.key.clone()),
                    );
                }
                Err(error) => tracing::warn!(%error, "could not refresh download status"),
            }
            let batch_active = queue.peek().items.iter().any(|item| {
                pending.contains(&item.id)
                    && matches!(
                        item.status,
                        DownloadStatus::Queued | DownloadStatus::Downloading
                    )
            });
            if !batch_active {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        }
        queue.write().active_job_id = None;
    }
    let mut state = queue.write();
    state.driver_running = false;
    state.active_job_id = None;
}

pub fn queue_downloads(requests: Vec<(String, String, String)>, mut queue: Signal<DownloadQueue>) {
    let api = consume_context::<Arc<dyn KopuzApi>>();
    let downloaded = consume_context::<DownloadedTracks>();
    let mut start_driver = false;
    {
        let mut state = queue.write();
        let known: HashSet<String> = state.items.iter().map(|item| item.id.clone()).collect();
        for (id, title, artist) in requests {
            if known.contains(&id) {
                continue;
            }
            state.items.push(DownloadItem {
                id,
                title,
                artist,
                status: DownloadStatus::Queued,
                bytes_done: 0,
                bytes_total: 0,
            });
        }
        if state.is_active() && !state.driver_running {
            state.driver_running = true;
            state.started_at = Some(Instant::now());
            start_driver = true;
        }
    }
    if start_driver {
        spawn_forever(drive_download_queue(api, queue, downloaded));
    }
}

pub fn delete_downloads(ids: Vec<String>, mut queue: Signal<DownloadQueue>) {
    let api = consume_context::<Arc<dyn KopuzApi>>();
    let mut downloaded = consume_context::<DownloadedTracks>();
    spawn_forever(async move {
        for id in ids {
            let _ = api.cancel_download_item(id.clone()).await;
            match api.remove_download(id.clone()).await {
                Ok(()) => {
                    queue.write().items.retain(|item| item.id != id);
                    downloaded.0.write().remove(&id);
                }
                Err(error) => tracing::warn!(%error, %id, "could not remove daemon download"),
            }
        }
    });
}

pub fn cancel_all_downloads(mut queue: Signal<DownloadQueue>) {
    let api = consume_context::<Arc<dyn KopuzApi>>();
    let (job_id, ids) = {
        let mut state = queue.write();
        let ids = state
            .items
            .iter_mut()
            .filter_map(|item| {
                matches!(
                    item.status,
                    DownloadStatus::Queued | DownloadStatus::Downloading
                )
                .then(|| {
                    item.status = DownloadStatus::Cancelled;
                    item.id.clone()
                })
            })
            .collect::<Vec<_>>();
        (state.active_job_id.clone(), ids)
    };
    spawn_forever(async move {
        if let Some(job_id) = job_id {
            let _ = api.cancel_job(job_id).await;
        }
        for id in ids {
            let _ = api.cancel_download_item(id).await;
        }
    });
}
