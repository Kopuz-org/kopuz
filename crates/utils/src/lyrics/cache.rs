use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Mutex, OnceLock};

use super::{LyricLine, Lyrics};

const LYRICS_CACHE_CAPACITY: usize = 256;
const LYRICS_META_KIND: &str = "lyrics";
const NEGATIVE_TTL_SECS: u64 = 7 * 24 * 60 * 60;

static LYRICS_CACHE: OnceLock<Mutex<LyricsCache>> = OnceLock::new();
static LYRICS_INFLIGHT: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

pub(super) struct LyricsCache {
    entries: HashMap<String, Option<Lyrics>>,
    order: VecDeque<String>,
    capacity: usize,
}

pub(super) struct LyricsInflightGuard {
    pub(super) key: String,
}

impl Drop for LyricsInflightGuard {
    fn drop(&mut self) {
        if let Ok(mut inflight) = lyrics_inflight().lock() {
            inflight.remove(&self.key);
        }
    }
}

impl LyricsCache {
    fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            capacity,
        }
    }

    pub(super) fn get_cloned(&mut self, key: &str) -> Option<Option<Lyrics>> {
        let value = self.entries.get(key).cloned()?;
        self.touch(key);
        Some(value)
    }

    pub(super) fn put(&mut self, key: String, value: Option<Lyrics>) {
        if self.entries.contains_key(&key) {
            self.entries.insert(key.clone(), value);
            self.touch(&key);
            return;
        }

        if self.entries.len() >= self.capacity {
            while let Some(oldest) = self.order.pop_front() {
                if self.entries.remove(&oldest).is_some() {
                    break;
                }
            }
        }

        self.order.push_back(key.clone());
        self.entries.insert(key, value);
    }

    fn touch(&mut self, key: &str) {
        if let Some(pos) = self.order.iter().position(|existing| existing == key) {
            self.order.remove(pos);
        }
        self.order.push_back(key.to_string());
    }
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn lyrics_to_payload(value: &Option<Lyrics>) -> String {
    let v = match value {
        Some(Lyrics::Synced(lines)) => serde_json::json!({
            "kind": "synced2",
            "lines": lines,
        }),
        Some(Lyrics::Plain(text)) => serde_json::json!({ "kind": "plain", "text": text }),
        None => serde_json::json!({ "kind": "none", "ts": now_unix() }),
    };
    v.to_string()
}

fn lyrics_from_payload(payload: &str) -> Option<Option<Lyrics>> {
    let v: serde_json::Value = serde_json::from_str(payload).ok()?;
    match v.get("kind").and_then(|k| k.as_str())? {
        "synced2" => {
            let lines: Vec<LyricLine> = serde_json::from_value(v.get("lines")?.clone()).ok()?;
            Some(Some(Lyrics::Synced(lines)))
        }
        "plain" => Some(Some(Lyrics::Plain(v.get("text")?.as_str()?.to_string()))),
        "none" => {
            let ts = v.get("ts").and_then(|t| t.as_u64()).unwrap_or(0);
            if now_unix().saturating_sub(ts) < NEGATIVE_TTL_SECS {
                Some(None)
            } else {
                None
            }
        }
        _ => None,
    }
}

pub(super) async fn load_persisted_lyrics(cache_key: &str) -> Option<Option<Lyrics>> {
    let handle = crate::db_cache::get()?;
    let payload = handle.meta_get(cache_key, LYRICS_META_KIND).await.ok()??;
    lyrics_from_payload(&payload)
}

pub(super) async fn store_lyrics(cache_key: &str, value: &Option<Lyrics>) {
    if let Ok(mut cache) = lyrics_cache().lock() {
        cache.put(cache_key.to_string(), value.clone());
    }
    if let Some(handle) = crate::db_cache::get() {
        let payload = lyrics_to_payload(value);
        let _ = handle.meta_put(cache_key, LYRICS_META_KIND, &payload).await;
    }
}

pub(super) fn lyrics_cache() -> &'static Mutex<LyricsCache> {
    LYRICS_CACHE.get_or_init(|| Mutex::new(LyricsCache::new(LYRICS_CACHE_CAPACITY)))
}

fn lyrics_inflight() -> &'static Mutex<HashSet<String>> {
    LYRICS_INFLIGHT.get_or_init(|| Mutex::new(HashSet::new()))
}

pub(super) fn try_begin_lyrics_fetch(key: &str) -> bool {
    let Ok(mut inflight) = lyrics_inflight().lock() else {
        return true;
    };

    if inflight.contains(key) {
        false
    } else {
        inflight.insert(key.to_string());
        true
    }
}
