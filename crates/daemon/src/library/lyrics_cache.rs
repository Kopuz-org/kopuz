//! Lyrics that survive a restart.
//!
//! The provider chain lives in `utils` and holds an in-process cache; what it
//! cannot do is persist, because the library is the daemon's and a frontend
//! linking SQLite is exactly what the split removed. So the read-through
//! around the fetch is here.
//!
//! A definitive "no words exist" is stored too, with a day's TTL: most of a
//! library has no lyrics anywhere, and without the negative entry every open
//! re-runs the whole provider chain over the network.

use utils::lyrics::{LyricLine, Lyrics};

use super::LibraryService;

const META_KIND: &str = "lyrics";
const NEGATIVE_TTL_SECS: u64 = 24 * 60 * 60;

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

fn to_payload(value: &Option<Lyrics>) -> String {
    let payload = match value {
        Some(Lyrics::Synced(lines)) => serde_json::json!({
            "kind": "synced2",
            "lines": lines,
        }),
        Some(Lyrics::Plain(text)) => serde_json::json!({ "kind": "plain", "text": text }),
        None => serde_json::json!({ "kind": "none", "ts": now_unix() }),
    };
    payload.to_string()
}

fn from_payload(payload: &str) -> Option<Option<Lyrics>> {
    let value: serde_json::Value = serde_json::from_str(payload).ok()?;
    match value.get("kind").and_then(|kind| kind.as_str())? {
        "synced2" => {
            let lines: Vec<LyricLine> = serde_json::from_value(value.get("lines")?.clone()).ok()?;
            Some(Some(Lyrics::Synced(lines)))
        }
        "plain" => Some(Some(Lyrics::Plain(
            value.get("text")?.as_str()?.to_string(),
        ))),
        // An expired miss reads as "nothing stored", so the providers run again.
        "none" => {
            let stored_at = value.get("ts").and_then(|ts| ts.as_u64()).unwrap_or(0);
            (now_unix().saturating_sub(stored_at) < NEGATIVE_TTL_SECS).then_some(None)
        }
        _ => None,
    }
}

impl LibraryService {
    pub(super) async fn persisted_lyrics(&self, cache_key: &str) -> Option<Option<Lyrics>> {
        let payload = self.db.meta_get(cache_key, META_KIND).await.ok()??;
        from_payload(&payload)
    }

    pub(super) async fn persist_lyrics(&self, cache_key: &str, value: &Option<Lyrics>) {
        if let Err(error) = self
            .db
            .meta_put(cache_key, META_KIND, &to_payload(value))
            .await
        {
            tracing::debug!(%error, "storing lyrics failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_miss_reads_back_as_a_known_absence() {
        let payload = to_payload(&None);
        assert_eq!(from_payload(&payload), Some(None));
    }

    #[test]
    fn an_expired_miss_reads_as_nothing_stored_so_the_providers_run_again() {
        let stale = now_unix() - NEGATIVE_TTL_SECS - 1;
        let payload = serde_json::json!({ "kind": "none", "ts": stale }).to_string();
        assert_eq!(from_payload(&payload), None);
    }

    #[test]
    fn plain_words_round_trip() {
        let value = Some(Lyrics::Plain("a line".into()));
        assert_eq!(from_payload(&to_payload(&value)), Some(value));
    }
}
