#[derive(Clone, Debug, PartialEq)]
pub enum DownloadStatus {
    Queued,
    Downloading,
    Done,
    Failed,
}

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

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
    cancelled_items: HashSet<String>,
}

impl DownloadQueue {
    pub fn cancel_item(&mut self, id: &str) {
        self.cancelled_items.insert(id.to_owned());
    }

    pub fn clear_item_cancellation(&mut self, id: &str) {
        self.cancelled_items.remove(id);
    }

    pub fn is_item_cancelled(&self, id: &str) -> bool {
        self.cancelled_items.contains(id)
    }

    pub fn is_active(&self) -> bool {
        self.items.iter().any(|i| {
            matches!(
                i.status,
                DownloadStatus::Queued | DownloadStatus::Downloading
            )
        })
    }

    pub fn current(&self) -> Option<&DownloadItem> {
        self.items
            .iter()
            .find(|i| matches!(i.status, DownloadStatus::Downloading))
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_cancellation_survives_removal_until_explicitly_requeued() {
        let mut queue = DownloadQueue::default();
        queue.cancel_item("track");
        assert!(queue.is_item_cancelled("track"));

        queue.clear_item_cancellation("track");
        assert!(!queue.is_item_cancelled("track"));
    }
}
