//! Queue persistence: each source's snapshot, stored as rows.

use async_trait::async_trait;

#[async_trait]
pub trait QueueStore: Send + Sync {
    /// The queue `source` was left with, empty when it has none; an error is a queue that could not be read.
    async fn load(&self, source: &config::Source) -> Result<db::QueueSnapshot, db::DbError>;
    async fn save(&self, source: &config::Source, snapshot: db::QueueSnapshot);
    /// Drop the queue stored for `source`.
    async fn forget(&self, source: &config::Source);
    /// The [`queue_fingerprint`] of what `source`'s server last accepted from this install.
    async fn synced_remote(&self, source: &config::Source) -> Option<String>;
    async fn set_synced_remote(&self, source: &config::Source, fingerprint: &str);
}

/// The `kv` kind a source's [`QueueStore::synced_remote`] is filed under.
const SYNCED_REMOTE_KIND: &str = "play_queue_synced";

/// What a server holds as a play queue, reduced to a value that tells our own last save from anyone else's.
/// Every Kopuz install signs its writes with the same client name, so `changedBy` cannot.
pub(crate) fn queue_fingerprint(
    ids: impl IntoIterator<Item = impl AsRef<str>>,
    current: Option<&str>,
    position_ms: u64,
) -> String {
    use sha2::{Digest as _, Sha256};
    use std::fmt::Write as _;
    let mut hasher = Sha256::new();
    for id in ids {
        hasher.update(id.as_ref().as_bytes());
        hasher.update([0]);
    }
    hasher.update([1]);
    hasher.update(current.unwrap_or_default().as_bytes());
    hasher.update([0]);
    hasher.update(whole_seconds(position_ms).to_le_bytes());
    let mut out = String::with_capacity(64);
    for byte in hasher.finalize() {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// A server may store the position at a coarser precision than it was sent at,
/// so both what is pushed and what is compared are whole seconds.
pub(crate) fn whole_seconds(position_ms: u64) -> u64 {
    position_ms.saturating_add(500) / 1000
}

/// Servers keep no position for an empty queue, so neither does its fingerprint.
pub(crate) fn empty_queue_fingerprint() -> String {
    queue_fingerprint([""; 0], None, 0)
}

/// The source, rows, shuffle and station records last written, so a save that only moved the playhead rewrites one row.
type Written = Option<(config::Source, Vec<reader::Track>, Vec<usize>, Stations)>;

type Stations = std::collections::BTreeMap<String, String>;

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
        if source.as_str().starts_with("removable:") {
            return Ok(db::QueueSnapshot::default());
        }
        self.db.load_queue(source).await
    }

    async fn forget(&self, source: &config::Source) {
        let mut written = self.written.lock().await;
        if written.as_ref().is_some_and(|(at, _, _, _)| at == source) {
            *written = None;
        }
        if let Err(error) = self.db.clear_queue(source).await {
            tracing::warn!(%error, "dropping a removed source's queue failed");
        }
    }

    async fn save(&self, source: &config::Source, snapshot: db::QueueSnapshot) {
        if source.as_str().starts_with("removable:") {
            return;
        }
        // Held across the write, so two saves never interleave their rows.
        let mut written = self.written.lock().await;
        let unchanged = written
            .as_ref()
            .is_some_and(|(at, queue, shuffle, stations)| {
                at == source
                    && *queue == snapshot.queue
                    && *shuffle == snapshot.shuffle_order
                    && *stations == snapshot.stations
            });
        let saved = match unchanged {
            true => self.db.save_queue_position(source, &snapshot).await,
            false => self.db.save_queue(source, &snapshot).await,
        };
        match saved {
            Ok(()) if !unchanged => {
                *written = Some((
                    source.clone(),
                    snapshot.queue,
                    snapshot.shuffle_order,
                    snapshot.stations,
                ));
            }
            Ok(()) => {}
            Err(error) => {
                *written = None;
                tracing::warn!(%error, "queue snapshot save failed");
            }
        }
    }

    async fn synced_remote(&self, source: &config::Source) -> Option<String> {
        match self.db.meta_get(source.as_str(), SYNCED_REMOTE_KIND).await {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                tracing::debug!(%error, "the last synced play queue could not be read");
                None
            }
        }
    }

    async fn set_synced_remote(&self, source: &config::Source, fingerprint: &str) {
        if let Err(error) = self
            .db
            .meta_put(source.as_str(), SYNCED_REMOTE_KIND, fingerprint)
            .await
        {
            tracing::warn!(%error, "the synced play queue could not be recorded");
        }
    }
}

/// Runs after the local restore, so a queue moved on another device wins.
pub async fn adopt_remote_queue(
    session: &crate::SessionHandle,
    library: &crate::LibraryService,
    source: &server::source::ActiveSource,
    store: &dyn QueueStore,
) {
    if !source.capabilities().play_queue || session.offline().offline_mode() {
        return;
    }
    // Anything the person does to the queue while the server answers wins over what it says.
    let queue_rev = session.state().queue.rev;
    let remote = match source.get_play_queue().await {
        Ok(remote) => remote,
        Err(error) => {
            tracing::debug!(%error, "saved play queue could not be read");
            return;
        }
    };
    let queried = source.source().clone();
    let synced = store.synced_remote(&queried).await;
    let changed_by = remote.as_ref().and_then(|remote| remote.changed_by.clone());
    let Some(snapshot) = remote_queue_to_adopt(remote, synced.as_deref()) else {
        return;
    };
    let adopted = snapshot.queue.len();
    // Network rows, not library ones: they resolve through the transient cache.
    library.register_transient(&snapshot.queue);
    match session.adopt_queue(snapshot, &queried, queue_rev).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            tracing::debug!("the queue changed while the server's was read; keeping it");
            return;
        }
        Err(error) => {
            tracing::warn!(%error, "remote play queue restore failed");
            return;
        }
    }
    tracing::info!(
        tracks = adopted,
        client = changed_by.as_deref().unwrap_or("another client"),
        "play queue adopted from the server"
    );
    session.emit_event(api::ApiEvent::Notice {
        level: api::NoticeLevel::Info,
        code: "play_queue_adopted".to_string(),
        message: changed_by,
    });
}

/// The server's queue as a snapshot to restore, unless it is still the one this install last saved there.
/// A server with no queue (`None`) holds an empty one.
fn remote_queue_to_adopt(
    remote: Option<server::source::RemotePlayQueue>,
    synced: Option<&str>,
) -> Option<db::QueueSnapshot> {
    let Some(remote) = remote.filter(|remote| !remote.tracks.is_empty()) else {
        // Only a queue this install synced can have been cleared since; a
        // server that never had one must not empty the local queue.
        let cleared = synced.is_some_and(|synced| synced != empty_queue_fingerprint());
        return cleared.then(db::QueueSnapshot::default);
    };
    let fingerprint = queue_fingerprint(
        remote.tracks.iter().map(|track| track.id.key()),
        remote.current_id.as_deref(),
        remote.position_ms,
    );
    if synced == Some(fingerprint.as_str()) {
        return None;
    }
    let current_queue_index = remote
        .current_id
        .as_deref()
        .and_then(|id| remote.tracks.iter().position(|track| track.id.key() == id))
        .unwrap_or(0);
    Some(db::QueueSnapshot {
        version: 1,
        queue: remote.tracks,
        current_queue_index,
        progress_secs: remote.position_ms / 1000,
        shuffle_order: Vec::new(),
        shuffle_enabled: false,
        radio_cursor: None,
        origin: None,
        stations: Default::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        DbQueueStore, QueueStore, empty_queue_fingerprint, queue_fingerprint, remote_queue_to_adopt,
    };

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
                counterpart: None,
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

    /// A playhead-only save writes the radio cursor too: a top-up that ends a
    /// radio changes it without touching the rows.
    #[tokio::test]
    async fn the_radio_cursor_survives_a_reload() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = db::init(&dir.path().join("queue.db")).await.expect("db");
        let store = DbQueueStore::new(db.clone());
        let source = config::Source::default();
        let radio = |cursor: Option<&str>| db::QueueSnapshot {
            version: 1,
            queue: queue(&["/a", "/b"]),
            radio_cursor: cursor.map(str::to_string),
            ..Default::default()
        };

        store.save(&source, radio(Some("RDAMVMx token"))).await;
        assert_eq!(
            store.load(&source).await.unwrap().radio_cursor.as_deref(),
            Some("RDAMVMx token")
        );

        store.save(&source, radio(None)).await;
        assert_eq!(store.load(&source).await.unwrap().radio_cursor, None);
    }

    /// Every install signs its saves `kopuz`, so only the last queue this one saved is its own.
    #[test]
    fn a_server_queue_is_adopted_unless_it_is_the_one_this_install_saved() {
        let remote = |position_ms| {
            Some(server::source::RemotePlayQueue {
                tracks: queue(&["/a", "/b"]),
                current_id: Some("/b".into()),
                position_ms,
                changed_by: Some("kopuz".into()),
            })
        };
        let ours = queue_fingerprint(["/a", "/b"], Some("/b"), 4000);

        assert!(remote_queue_to_adopt(remote(4000), Some(&ours)).is_none());

        let moved_on = remote_queue_to_adopt(remote(9000), Some(&ours)).expect("adopted");
        assert_eq!(
            (moved_on.current_queue_index, moved_on.progress_secs),
            (1, 9)
        );
        assert!(remote_queue_to_adopt(remote(4000), None).is_some());
        assert!(remote_queue_to_adopt(remote(4400), Some(&ours)).is_none());
    }

    /// A server answers a cleared queue with none at all, or an empty one.
    #[test]
    fn a_cleared_server_queue_empties_only_a_queue_this_install_synced() {
        let ours = queue_fingerprint(["/a", "/b"], Some("/b"), 4000);
        let empty = || {
            Some(server::source::RemotePlayQueue {
                tracks: Vec::new(),
                current_id: None,
                position_ms: 0,
                changed_by: Some("Arpeggi".into()),
            })
        };

        let cleared = remote_queue_to_adopt(None, Some(&ours)).expect("cleared elsewhere");
        assert!(cleared.queue.is_empty());
        assert!(remote_queue_to_adopt(empty(), Some(&ours)).is_some());

        assert!(remote_queue_to_adopt(None, None).is_none());
        assert!(remote_queue_to_adopt(empty(), None).is_none());
        let cleared_here = empty_queue_fingerprint();
        assert!(remote_queue_to_adopt(None, Some(&cleared_here)).is_none());
    }
}
