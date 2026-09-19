//! Queue persistence: the daemon-side home of the snapshot the app crate
//! saves via `queue_state.rs`. Same `db::QueueSnapshot` row, same version and
//! progress rounding, so the GUI and the daemon can restore each other's
//! sessions during the transition.

use async_trait::async_trait;

#[async_trait]
pub trait QueueStore: Send + Sync {
    async fn load(&self) -> Option<db::QueueSnapshot>;
    async fn save(&self, snapshot: db::QueueSnapshot);
}

pub struct DbQueueStore {
    db: db::Db,
}

impl DbQueueStore {
    pub fn new(db: db::Db) -> Self {
        Self { db }
    }
}

#[async_trait]
impl QueueStore for DbQueueStore {
    async fn load(&self) -> Option<db::QueueSnapshot> {
        match self.db.load_queue().await {
            Ok(snapshot) => Some(snapshot),
            Err(error) => {
                tracing::warn!(%error, "queue snapshot load failed");
                None
            }
        }
    }

    async fn save(&self, snapshot: db::QueueSnapshot) {
        if let Err(error) = self.db.save_queue(&snapshot).await {
            tracing::warn!(%error, "queue snapshot save failed");
        }
    }
}

/// Adopt the active source's own saved play queue when another client wrote it
/// last. Runs after the local snapshot restore, so a queue moved on another
/// device wins over the one this machine left behind.
pub async fn adopt_remote_queue(
    session: &crate::SessionHandle,
    library: &crate::LibraryService,
    source: &server::source::ActiveSource,
) {
    if !source.capabilities().play_queue {
        return;
    }
    let remote = match source.get_play_queue().await {
        Ok(Some(remote)) => remote,
        Ok(None) => return,
        Err(error) => {
            tracing::debug!(%error, "saved play queue could not be read");
            return;
        }
    };
    if remote.tracks.is_empty() || !remote.changed_elsewhere {
        return;
    }

    let current_queue_index = remote
        .current_id
        .as_deref()
        .and_then(|id| remote.tracks.iter().position(|track| track.id.key() == id))
        .unwrap_or(0);
    let snapshot = db::QueueSnapshot {
        version: 1,
        queue: remote.tracks,
        current_queue_index,
        progress_secs: remote.position_ms / 1000,
        shuffle_order: Vec::new(),
        shuffle_enabled: false,
    };
    let adopted = snapshot.queue.len();
    // These rows come off the network, not the library, so hearting or
    // queueing one has to resolve through the transient cache.
    library.register_transient(&snapshot.queue);
    if let Err(error) = session.restore_queue(snapshot).await {
        tracing::warn!(%error, "remote play queue restore failed");
        return;
    }
    tracing::info!(
        tracks = adopted,
        client = remote.changed_by.as_deref().unwrap_or("another client"),
        "play queue adopted from the server"
    );
    session.emit_event(api::ApiEvent::Notice {
        level: api::NoticeLevel::Info,
        code: "play_queue_adopted".to_string(),
        message: remote.changed_by,
    });
}
