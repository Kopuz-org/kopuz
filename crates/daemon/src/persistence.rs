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
