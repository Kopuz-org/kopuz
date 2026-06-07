#[derive(Clone, Debug, PartialEq)]
pub enum DownloadStatus {
    Queued,
    Downloading,
    Done,
    Failed,
}

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Debug, Default)]
pub struct DownloadProgress {
    pub per_item: HashMap<String, u64>,
    pub bytes_done_session: u64,
    pub session_elapsed_secs: f64,
}

#[derive(Clone, Debug)]
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
    pub is_running: bool,
    pub cancel_requested: bool,
    pub cancel_flag: Arc<AtomicBool>,
    pub bytes_done_session: u64,
    pub session_elapsed_secs: f64,
}

impl DownloadQueue {
    pub fn is_active(&self) -> bool {
        self.items.iter().any(|i| {
            matches!(
                i.status,
                DownloadStatus::Queued | DownloadStatus::Downloading
            )
        })
    }

    pub fn done_count(&self) -> usize {
        self.items
            .iter()
            .filter(|i| matches!(i.status, DownloadStatus::Done))
            .count()
    }

    pub fn total_non_failed(&self) -> usize {
        self.items
            .iter()
            .filter(|i| !matches!(i.status, DownloadStatus::Failed))
            .count()
    }

    pub fn current(&self) -> Option<&DownloadItem> {
        self.items
            .iter()
            .find(|i| matches!(i.status, DownloadStatus::Downloading))
    }

    pub fn eta_secs(&self) -> Option<u64> {
        let elapsed = self.session_elapsed_secs;
        if elapsed < 0.5 || self.bytes_done_session == 0 {
            return None;
        }
        let bps = self.bytes_done_session as f64 / elapsed;

        let avg_size: u64 = {
            let known: Vec<u64> = self
                .items
                .iter()
                .filter(|i| i.bytes_total > 0)
                .map(|i| i.bytes_total)
                .collect();
            if known.is_empty() {
                8_000_000
            } else {
                known.iter().sum::<u64>() / known.len() as u64
            }
        };

        let remaining: u64 = self
            .items
            .iter()
            .filter(|i| {
                matches!(
                    i.status,
                    DownloadStatus::Queued | DownloadStatus::Downloading
                )
            })
            .map(|i| {
                if i.bytes_total > 0 {
                    i.bytes_total.saturating_sub(i.bytes_done)
                } else {
                    avg_size
                }
            })
            .sum();

        if bps > 0.0 {
            Some((remaining as f64 / bps) as u64)
        } else {
            None
        }
    }

    pub fn dismiss(&mut self) {
        self.items.retain(|i| {
            matches!(
                i.status,
                DownloadStatus::Queued | DownloadStatus::Downloading
            )
        });
        if self.items.is_empty() {
            self.bytes_done_session = 0;
            self.session_elapsed_secs = 0.0;
        }
    }

    pub fn cancel_all(&mut self) {
        self.cancel_requested = true;
        self.cancel_flag.store(true, Ordering::Relaxed);
        for item in &mut self.items {
            if matches!(
                item.status,
                DownloadStatus::Queued | DownloadStatus::Downloading
            ) {
                item.status = DownloadStatus::Failed;
            }
        }
    }
}
