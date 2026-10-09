//! Queue persistence: each source's snapshot, stored as rows.

use async_trait::async_trait;

#[async_trait]
pub trait QueueStore: Send + Sync {
    /// The queue `source` was left with, empty when it has none; an error is a queue that could not be read.
    async fn load(&self, source: &config::Source) -> Result<db::QueueSnapshot, db::DbError>;
    async fn save(&self, source: &config::Source, snapshot: db::QueueSnapshot);
    /// Drop the queue stored for `source`.
    async fn forget(&self, source: &config::Source);
}

/// The source, rows and shuffle last written, so a save that only moved the playhead rewrites one row.
type Written = Option<(config::Source, Vec<reader::Track>, Vec<usize>)>;

pub struct DbQueueStore {
    db: db::Db,
    written: tokio::sync::Mutex<Written>,
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
    async fn load(&self, source: &config::Source) -> Result<db::QueueSnapshot, db::DbError> {
        self.db.load_queue(source).await
    }

    async fn forget(&self, source: &config::Source) {
        let mut written = self.written.lock().await;
        if written.as_ref().is_some_and(|(at, _, _)| at == source) {
            *written = None;
        }
        if let Err(error) = self.db.clear_queue(source).await {
            tracing::warn!(%error, "dropping a removed source's queue failed");
        }
    }

    async fn save(&self, source: &config::Source, snapshot: db::QueueSnapshot) {
        // Held across the write, so two saves never interleave their rows.
        let mut written = self.written.lock().await;
        let unchanged = written.as_ref().is_some_and(|(at, queue, shuffle)| {
            at == source && *queue == snapshot.queue && *shuffle == snapshot.shuffle_order
        });
        let saved = match unchanged {
            true => self.db.save_queue_position(source, &snapshot).await,
            false => self.db.save_queue(source, &snapshot).await,
        };
        match saved {
            Ok(()) if !unchanged => {
                *written = Some((source.clone(), snapshot.queue, snapshot.shuffle_order));
            }
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
                replay_gain: config::ReplayGainInfo::default(),
                credits: Vec::new(),
                explicit: false,
                plays: None,
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
        let source = config::Source::default();
        let playing = |keys: &[&str], progress_secs| db::QueueSnapshot {
            version: 1,
            queue: queue(keys),
            progress_secs,
            ..Default::default()
        };
        store.save(&source, playing(&["/a", "/b"], 0)).await;
        // Rows the store did not write, so a rewrite would show.
        db.save_queue(&source, &playing(&["/elsewhere"], 0))
            .await
            .unwrap();

        store.save(&source, playing(&["/a", "/b"], 5)).await;
        let stored = db.load_queue(&source).await.unwrap();
        assert_eq!(
            (titles(&stored), stored.progress_secs),
            (vec!["/elsewhere"], 5)
        );

        store.save(&source, playing(&["/a", "/c"], 5)).await;
        assert_eq!(titles(&db.load_queue(&source).await.unwrap()), ["/a", "/c"]);
    }
}
