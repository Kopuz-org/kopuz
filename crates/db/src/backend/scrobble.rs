//! Offline scrobble domain types (issue #335). `db` owns the `scrobble_queue`
//! table these describe, so the enum and row live here; `kopuz-scrobble`
//! re-exports them so its Last.fm/Libre.fm/ListenBrainz backends and the queue
//! share one representation. This crate only persists — the retry
//! orchestration lives in `kopuz-scrobble`.

/// A scrobble destination. Kept in sync with the `service` column tags below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrobbleService {
    LastFm,
    LibreFm,
    ListenBrainz,
}

impl ScrobbleService {
    pub fn label(self) -> &'static str {
        match self {
            ScrobbleService::LastFm => "Last.fm",
            ScrobbleService::LibreFm => "Libre.fm",
            ScrobbleService::ListenBrainz => "ListenBrainz",
        }
    }

    pub fn as_tag(self) -> &'static str {
        match self {
            ScrobbleService::LastFm => "lastfm",
            ScrobbleService::LibreFm => "librefm",
            ScrobbleService::ListenBrainz => "listenbrainz",
        }
    }

    pub fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "lastfm" => Some(ScrobbleService::LastFm),
            "librefm" => Some(ScrobbleService::LibreFm),
            "listenbrainz" => Some(ScrobbleService::ListenBrainz),
            _ => None,
        }
    }
}

/// One queued offline scrobble owed to a single service (issue #335). A listen
/// owed to N services is N rows sharing `(listened_at, artist, title)`.
/// `listen_info` is the raw ListenBrainz additional-info JSON, `None` otherwise.
/// The retry orchestration lives in `kopuz-scrobble` — this crate only persists.
#[derive(Clone, Debug)]
pub struct QueuedScrobbleRow {
    pub listened_at: i64,
    pub artist: String,
    pub title: String,
    pub album: Option<String>,
    pub service: ScrobbleService,
    pub listen_info: Option<String>,
}
