//! Offline scrobble queue (issue #335).
//!
//! A scrobble that fails with a transient error (no response or a 5xx) is
//! persisted here and resubmitted later with its original listen timestamp.
//! Both protocols support backdated submissions: Last.fm/Libre.fm
//! `track.scrobble` takes a `timestamp`, ListenBrainz takes `listened_at`.
//! Permanent failures (4xx, e.g. bad credentials) are never queued, retrying
//! them can't succeed.
//!
//! Persistence lives in `kopuz-db` (the `scrobble_queue` table); this module
//! is the retry orchestration on top of it. One listen owed to several services
//! is several rows sharing a timestamp, so delivering one service never blocks
//! the others.

use crate::{lastfm, librefm, musicbrainz};
use db::{Db, QueuedScrobbleRow};
use std::collections::HashMap;

pub use db::Service;

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

/// Enqueue a failed scrobble for the given service. A repeat of the same listen
/// on the same service folds into the existing row (see `scrobble_queue_push`),
/// so one track failing on two services is two rows, one per service.
pub async fn enqueue(
    db: &Db,
    service: Service,
    artist: &str,
    title: &str,
    album: Option<&str>,
    timestamp: i64,
    listen_info: Option<serde_json::Map<String, serde_json::Value>>,
) {
    let listen_info = listen_info.and_then(|m| serde_json::to_string(&m).ok());
    let row = QueuedScrobbleRow {
        listened_at: timestamp,
        artist: artist.to_string(),
        title: title.to_string(),
        album: album.map(str::to_string),
        service,
        listen_info,
    };

    if let Err(e) = db.scrobble_queue_push(&row).await {
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
/// failure drops the row. Each delivered row is removed immediately, so an
/// interrupted drain re-sends at most the one in-flight item instead of
/// replaying everything already delivered — and `enqueue()` calls that arrive
/// mid-drain simply insert new rows the current pass won't touch.
pub async fn drain(db: &Db, creds: &Credentials) {
    let rows = match db.scrobble_queue_all().await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "failed to load scrobble queue");
            return;
        }
    };
    if rows.is_empty() {
        return;
    }
    tracing::info!("draining scrobble queue ({} items)", rows.len());

    let mut give_up: Vec<Service> = Vec::new();

    for row in &rows {
        let service = row.service;
        if give_up.contains(&service) {
            continue;
        }

        let delivered = match submit_one(service, row, creds).await {
            Outcome::Sent => {
                tracing::info!(
                    service = service.label(),
                    "resubmitted queued scrobble: {} - {}",
                    row.artist,
                    row.title
                );
                true
            }
            Outcome::NoCredentials | Outcome::Transient => {
                give_up.push(service);
                false
            }
            Outcome::Permanent(e) => {
                tracing::warn!(
                    service = service.label(),
                    error = %e,
                    "dropping queued scrobble after permanent error: {} - {}",
                    row.artist,
                    row.title
                );
                true
            }
        };

        if delivered
            && let Err(e) = db
                .scrobble_queue_delete(row.listened_at, &row.artist, &row.title, service)
                .await
        {
            tracing::warn!(error = %e, "failed to checkpoint scrobble queue");
        }
    }
}

enum Outcome {
    Sent,
    Transient,
    Permanent(reqwest::Error),
    NoCredentials,
}

async fn submit_one(service: Service, item: &QueuedScrobbleRow, creds: &Credentials) -> Outcome {
    let album = item.album.as_deref();
    let result = match service {
        Service::LastFm => {
            let Some((key, secret, session)) = &creds.lastfm else {
                return Outcome::NoCredentials;
            };
            let scrobble =
                lastfm::make_scrobble_at(&item.artist, &item.title, album, item.listened_at);
            lastfm::submit_scrobble(key, secret, session, &scrobble)
                .await
                .map(|_| ())
        }
        Service::LibreFm => {
            let Some(session) = &creds.librefm_session_key else {
                return Outcome::NoCredentials;
            };
            let scrobble =
                librefm::make_scrobble_at(&item.artist, &item.title, album, item.listened_at);
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
            let parsed: Option<serde_json::Map<String, serde_json::Value>> = item
                .listen_info
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok());
            let info: Option<HashMap<&str, serde_json::Value>> = parsed
                .as_ref()
                .map(|m| m.iter().map(|(k, v)| (k.as_str(), v.clone())).collect());
            let listen =
                musicbrainz::make_listen(&item.artist, &item.title, album, info, item.listened_at);
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

    /// A fresh isolated on-disk DB per test (WAL sidecar files cleaned up too).
    struct TempDb {
        db: Db,
        path: std::path::PathBuf,
    }

    impl Drop for TempDb {
        fn drop(&mut self) {
            for ext in ["", "-wal", "-shm"] {
                let mut p = self.path.clone().into_os_string();
                p.push(ext);
                let _ = std::fs::remove_file(p);
            }
        }
    }

    async fn temp_db(name: &str) -> TempDb {
        let path =
            std::env::temp_dir().join(format!("kopuz-queue-test-{}-{name}.db", std::process::id()));
        for ext in ["", "-wal", "-shm"] {
            let mut p = path.clone().into_os_string();
            p.push(ext);
            let _ = std::fs::remove_file(p);
        }
        let db = db::init(&path).await.unwrap();
        TempDb { db, path }
    }

    #[tokio::test]
    async fn enqueue_merges_same_listen_across_services() {
        let t = temp_db("merge").await;

        enqueue(
            &t.db,
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
            &t.db,
            Service::ListenBrainz,
            "Artist",
            "Song",
            Some("Album"),
            42,
            Some(info.clone()),
        )
        .await;
        enqueue(
            &t.db,
            Service::LastFm,
            "Artist",
            "Song",
            Some("Album"),
            42,
            None,
        )
        .await;

        let rows = t.db.scrobble_queue_all().await.unwrap();
        assert_eq!(rows.len(), 2);
        let services: Vec<Service> = rows.iter().map(|r| r.service).collect();
        assert!(services.contains(&Service::LastFm));
        assert!(services.contains(&Service::ListenBrainz));
        let lb = rows
            .iter()
            .find(|r| r.service == Service::ListenBrainz)
            .unwrap();
        assert_eq!(lb.listen_info.as_deref(), Some(r#"{"duration_ms":180000}"#));
    }

    #[tokio::test]
    async fn drain_without_credentials_retains_queue() {
        let t = temp_db("nocreds").await;
        enqueue(&t.db, Service::LastFm, "Artist", "Song", None, 42, None).await;

        drain(&t.db, &Credentials::default()).await;

        assert_eq!(t.db.scrobble_queue_all().await.unwrap().len(), 1);
    }
}
