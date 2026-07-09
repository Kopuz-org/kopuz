//! Offline scrobble queue (issue #335).
//!
//! A scrobble that fails with a transient error (no response or a 5xx) is
//! persisted here and resubmitted later with its original listen timestamp.
//! Both protocols support backdated submissions: Last.fm/Libre.fm
//! `track.scrobble` takes a `timestamp`, ListenBrainz takes `listened_at`.
//! Permanent failures (4xx, e.g. bad credentials) are never queued, retrying
//! them can't succeed.

use crate::{lastfm, librefm, musicbrainz};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::sync::Mutex;

/// Queue size cap; the oldest entries are dropped first.
const MAX_QUEUED: usize = 500;

/// Serializes all queue file access so two finished tracks can't race on the
/// load-modify-save cycle.
static QUEUE_LOCK: Mutex<()> = Mutex::const_new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Service {
    LastFm,
    LibreFm,
    ListenBrainz,
}

impl Service {
    pub fn label(self) -> &'static str {
        match self {
            Service::LastFm => "Last.fm",
            Service::LibreFm => "Libre.fm",
            Service::ListenBrainz => "ListenBrainz",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedScrobble {
    pub artist: String,
    pub title: String,
    pub album: Option<String>,
    /// Unix timestamp of the original listen.
    pub timestamp: i64,
    /// Services this scrobble is still owed to.
    pub pending: Vec<Service>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listen_info: Option<serde_json::Map<String, serde_json::Value>>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ScrobbleQueue {
    pub items: Vec<QueuedScrobble>,
}

impl ScrobbleQueue {
    /// Load the queue; a missing or unreadable file is an empty queue
    /// (same forgiving pattern as the app config).
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Write-to-temp-then-rename so an interrupted write can't leave a
    /// truncated file behind; `load` treats corrupt JSON as an empty queue,
    /// which would silently drop the whole backlog.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string(self).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    }

    /// Append a scrobble, dropping the oldest entries beyond the cap.
    pub fn push(&mut self, item: QueuedScrobble) {
        self.items.push(item);
        if self.items.len() > MAX_QUEUED {
            let overflow = self.items.len() - MAX_QUEUED;
            self.items.drain(..overflow);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// Default queue location, next to config.json.
pub fn default_queue_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("com", "temidaradev", "kopuz")
        .map(|dirs| dirs.config_dir().join("scrobble_queue.json"))
}

/// Whether a failed request is worth retrying later: no response at all
/// (offline, DNS, timeout) or a server-side 5xx. A 4xx means the server
/// understood and refused (bad credentials etc.), retrying can't help.
pub fn is_transient(error: &reqwest::Error) -> bool {
    match error.status() {
        Some(status) => status.is_server_error(),
        None => true,
    }
}

/// Credentials snapshot for draining; `None` skips that service.
#[derive(Debug, Clone, Default)]
pub struct Credentials {
    /// (api_key, api_secret, session_key)
    pub lastfm: Option<(String, String, String)>,
    pub librefm_session_key: Option<String>,
    /// Raw token as stored in config; "Token " prefix added when missing.
    pub listenbrainz_token: Option<String>,
}

/// Enqueue a failed scrobble for the given service, merging with an existing
/// entry for the same listen so one track failing on two services stays one
/// queue item with two pending services.
pub async fn enqueue(
    path: &Path,
    service: Service,
    artist: &str,
    title: &str,
    album: Option<&str>,
    timestamp: i64,
    listen_info: Option<serde_json::Map<String, serde_json::Value>>,
) {
    let _guard = QUEUE_LOCK.lock().await;
    let mut queue = ScrobbleQueue::load(path);

    if let Some(existing) = queue
        .items
        .iter_mut()
        .find(|i| i.timestamp == timestamp && i.artist == artist && i.title == title)
    {
        if !existing.pending.contains(&service) {
            existing.pending.push(service);
        }
        if listen_info.is_some() {
            existing.listen_info = listen_info;
        }
    } else {
        queue.push(QueuedScrobble {
            artist: artist.to_string(),
            title: title.to_string(),
            album: album.map(|s| s.to_string()),
            timestamp,
            pending: vec![service],
            listen_info,
        });
    }

    if let Err(e) = queue.save(path) {
        tracing::warn!(error = %e, "failed to persist scrobble queue");
    } else {
        tracing::info!(
            service = service.label(),
            "queued offline scrobble: {} - {}",
            artist,
            title
        );
    }
}

/// Resubmit queued scrobbles. Per service, the first transient failure skips
/// that service for the rest of this run (still unreachable); a permanent
/// failure drops the service from that entry. Progress is checkpointed to
/// disk after every entry, so an interrupted drain re-sends at most the one
/// in-flight item instead of replaying everything already delivered.
///
/// The mutex is held only around file I/O; network submissions run outside
/// the lock so concurrent `enqueue()` calls are never blocked by the wire.
/// Each checkpoint re-loads the on-disk queue and merges progress into it,
/// preserving any entries added by `enqueue()` while drain was in flight.
pub async fn drain(path: &Path, creds: &Credentials) {
    let queue = {
        let _guard = QUEUE_LOCK.lock().await;
        ScrobbleQueue::load(path)
    };
    if queue.is_empty() {
        return;
    }
    tracing::info!("draining scrobble queue ({} items)", queue.items.len());

    let mut give_up: Vec<Service> = Vec::new();

    for item in &queue.items {
        let mut done: Vec<Service> = Vec::new();
        for &service in &item.pending {
            if give_up.contains(&service) {
                continue;
            }
            match submit_one(service, item, creds).await {
                Outcome::Sent => {
                    tracing::info!(
                        service = service.label(),
                        "resubmitted queued scrobble: {} - {}",
                        item.artist,
                        item.title
                    );
                    done.push(service);
                }
                Outcome::NoCredentials => {
                    // Not configured (anymore); nothing to deliver to.
                    done.push(service);
                }
                Outcome::Transient => {
                    give_up.push(service);
                }
                Outcome::Permanent(e) => {
                    tracing::warn!(
                        service = service.label(),
                        error = %e,
                        "dropping queued scrobble after permanent error: {} - {}",
                        item.artist,
                        item.title
                    );
                    done.push(service);
                }
            }
        }
        if !done.is_empty() {
            // Re-acquire the lock only for the checkpoint write. Reload the
            // on-disk queue to merge any enqueue() calls that arrived while
            // we were on the wire, then remove the services we just delivered.
            let _guard = QUEUE_LOCK.lock().await;
            let mut on_disk = ScrobbleQueue::load(path);
            if let Some(disk_item) = on_disk.items.iter_mut().find(|i| {
                i.timestamp == item.timestamp && i.artist == item.artist && i.title == item.title
            }) {
                disk_item.pending.retain(|s| !done.contains(s));
            }
            on_disk.items.retain(|i| !i.pending.is_empty());
            if let Err(e) = on_disk.save(path) {
                tracing::warn!(error = %e, "failed to checkpoint scrobble queue");
            }
        }
    }
}

enum Outcome {
    Sent,
    Transient,
    Permanent(reqwest::Error),
    NoCredentials,
}

async fn submit_one(service: Service, item: &QueuedScrobble, creds: &Credentials) -> Outcome {
    let album = item.album.as_deref();
    let result = match service {
        Service::LastFm => {
            let Some((key, secret, session)) = &creds.lastfm else {
                return Outcome::NoCredentials;
            };
            let scrobble =
                lastfm::make_scrobble_at(&item.artist, &item.title, album, item.timestamp);
            lastfm::submit_scrobble(key, secret, session, &scrobble)
                .await
                .map(|_| ())
        }
        Service::LibreFm => {
            let Some(session) = &creds.librefm_session_key else {
                return Outcome::NoCredentials;
            };
            let scrobble =
                librefm::make_scrobble_at(&item.artist, &item.title, album, item.timestamp);
            librefm::submit_scrobble(librefm::API_KEY, librefm::API_SECRET, session, &scrobble)
                .await
                .map(|_| ())
        }
        Service::ListenBrainz => {
            let Some(token) = &creds.listenbrainz_token else {
                return Outcome::NoCredentials;
            };
            let auth = if token.contains(' ') {
                token.clone()
            } else {
                format!("Token {token}")
            };
            let info: Option<HashMap<&str, serde_json::Value>> = item
                .listen_info
                .as_ref()
                .map(|m| m.iter().map(|(k, v)| (k.as_str(), v.clone())).collect());
            let listen =
                musicbrainz::make_listen(&item.artist, &item.title, album, info, item.timestamp);
            musicbrainz::submit_listens(&auth, vec![listen], "import")
                .await
                .map(|_| ())
        }
    };

    match result {
        Ok(()) => Outcome::Sent,
        Err(e) if is_transient(&e) => Outcome::Transient,
        Err(e) => Outcome::Permanent(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(title: &str, ts: i64, pending: Vec<Service>) -> QueuedScrobble {
        QueuedScrobble {
            artist: "Artist".into(),
            title: title.into(),
            album: None,
            timestamp: ts,
            pending,
            listen_info: None,
        }
    }

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("kopuz-queue-test-{}-{name}", std::process::id()))
    }

    #[test]
    fn push_caps_queue_and_drops_oldest() {
        let mut q = ScrobbleQueue::default();
        for i in 0..(MAX_QUEUED + 10) {
            q.push(entry(&format!("t{i}"), i as i64, vec![Service::LastFm]));
        }
        assert_eq!(q.items.len(), MAX_QUEUED);
        // The 10 oldest entries were dropped.
        assert_eq!(q.items.first().unwrap().title, "t10");
    }

    #[test]
    fn save_and_load_roundtrip() {
        let path = temp_path("roundtrip.json");
        let mut q = ScrobbleQueue::default();
        q.push(entry(
            "song",
            1700000000,
            vec![Service::LibreFm, Service::ListenBrainz],
        ));
        q.save(&path).unwrap();

        let loaded = ScrobbleQueue::load(&path);
        assert_eq!(loaded.items.len(), 1);
        assert_eq!(loaded.items[0].title, "song");
        assert_eq!(
            loaded.items[0].pending,
            vec![Service::LibreFm, Service::ListenBrainz]
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn corrupt_or_missing_file_loads_empty() {
        let missing = temp_path("missing.json");
        assert!(ScrobbleQueue::load(&missing).is_empty());

        let corrupt = temp_path("corrupt.json");
        std::fs::write(&corrupt, "not json at all").unwrap();
        assert!(ScrobbleQueue::load(&corrupt).is_empty());
        let _ = std::fs::remove_file(&corrupt);
    }

    #[tokio::test]
    async fn enqueue_merges_same_listen_across_services() {
        let path = temp_path("merge.json");
        let _ = std::fs::remove_file(&path);

        enqueue(
            &path,
            Service::LastFm,
            "Artist",
            "Song",
            Some("Album"),
            42,
            None,
        )
        .await;
        let mut info = serde_json::Map::new();
        info.insert("duration_ms".into(), serde_json::Value::from(180000));
        enqueue(
            &path,
            Service::ListenBrainz,
            "Artist",
            "Song",
            Some("Album"),
            42,
            Some(info.clone()),
        )
        .await;
        // Duplicate service on the same listen must not double up.
        enqueue(
            &path,
            Service::LastFm,
            "Artist",
            "Song",
            Some("Album"),
            42,
            None,
        )
        .await;

        let q = ScrobbleQueue::load(&path);
        assert_eq!(q.items.len(), 1);
        assert_eq!(
            q.items[0].pending,
            vec![Service::LastFm, Service::ListenBrainz]
        );
        assert_eq!(q.items[0].listen_info, Some(info));
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn drain_without_credentials_clears_queue() {
        // No credentials configured: nothing to deliver to, entries are
        // released instead of sitting in the queue forever.
        let path = temp_path("nocreds.json");
        let _ = std::fs::remove_file(&path);
        enqueue(&path, Service::LastFm, "Artist", "Song", None, 42, None).await;

        drain(&path, &Credentials::default()).await;

        assert!(ScrobbleQueue::load(&path).is_empty());
        let _ = std::fs::remove_file(&path);
    }
}
