//! Queue persistence: the session's snapshot, stored as rows.

use async_trait::async_trait;

#[async_trait]
pub trait QueueStore: Send + Sync {
    async fn load(&self) -> Option<db::QueueSnapshot>;
    async fn save(&self, snapshot: db::QueueSnapshot);
}

pub struct DbQueueStore {
    db: db::Db,
    /// The rows and shuffle last written, so a save that only moved the playhead rewrites one row.
    written: tokio::sync::Mutex<Option<(Vec<reader::Track>, Vec<usize>)>>,
}

impl DbQueueStore {
    pub fn new(db: db::Db) -> Self {
        Self {
            db,
            written: tokio::sync::Mutex::new(None),
        }
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
        // Held across the write, so two saves never interleave their rows.
        let mut written = self.written.lock().await;
        let unchanged = written.as_ref().is_some_and(|(queue, shuffle)| {
            *queue == snapshot.queue && *shuffle == snapshot.shuffle_order
        });
        let saved = match unchanged {
            true => self.db.save_queue_position(&snapshot).await,
            false => self.db.save_queue(&snapshot).await,
        };
        match saved {
            Ok(()) if !unchanged => *written = Some((snapshot.queue, snapshot.shuffle_order)),
            Ok(()) => {}
            Err(error) => {
                *written = None;
                tracing::warn!(%error, "queue snapshot save failed");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DbQueueStore, QueueStore};

    fn queue(keys: &[&str]) -> Vec<reader::Track> {
        keys.iter()
            .map(|key| reader::Track {
                id: reader::TrackId::Local(std::path::PathBuf::from(key)),
                cover: None,
                album_id: String::new(),
                title: (*key).into(),
                artist: String::new(),
                album: String::new(),
                duration: 60,
                khz: 44,
                bitrate: 320,
                track_number: None,
                disc_number: None,
                musicbrainz_release_id: None,
                musicbrainz_recording_id: None,
                musicbrainz_track_id: None,
                playlist_item_id: None,
                artists: Vec::new(),
                credits: Vec::new(),
            })
            .collect()
    }

    fn titles(snapshot: &db::QueueSnapshot) -> Vec<&str> {
        snapshot.queue.iter().map(|t| t.title.as_str()).collect()
    }

    /// Progress ticks every few seconds while playing, so a save that only moved the playhead must not rewrite every row.
    #[tokio::test]
    async fn only_a_changed_list_rewrites_the_rows() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = db::init(&dir.path().join("queue.db")).await.expect("db");
        let store = DbQueueStore::new(db.clone());
        let playing = |keys: &[&str], progress_secs| db::QueueSnapshot {
            version: 1,
            queue: queue(keys),
            progress_secs,
            ..Default::default()
        };
        store.save(playing(&["/a", "/b"], 0)).await;
        // Rows the store did not write, so a rewrite would show.
        db.save_queue(&playing(&["/elsewhere"], 0)).await.unwrap();

        store.save(playing(&["/a", "/b"], 5)).await;
        let stored = db.load_queue().await.unwrap();
        assert_eq!(
            (titles(&stored), stored.progress_secs),
            (vec!["/elsewhere"], 5)
        );

        store.save(playing(&["/a", "/c"], 5)).await;
        assert_eq!(titles(&db.load_queue().await.unwrap()), ["/a", "/c"]);
    }
}
