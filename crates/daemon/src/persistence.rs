//! Queue persistence shared by headless sessions and the frontend API. Both
//! paths write the same snapshot row and use the same progress rounding.

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
