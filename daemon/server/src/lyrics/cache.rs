use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Mutex, OnceLock};

use super::Lyrics;

const LYRICS_CACHE_CAPACITY: usize = 256;

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

/// Hold an answer for this process. Surviving a restart is the daemon's job:
/// it owns the library the words are written to.
pub(super) fn remember_lyrics(cache_key: &str, value: &Option<Lyrics>) {
    if let Ok(mut cache) = lyrics_cache().lock() {
        cache.put(cache_key.to_string(), value.clone());
    }
}

/// Seed the in-memory cache from whatever the caller persisted.
pub fn prime(cache_key: &str, value: Option<Lyrics>) {
    if let Ok(mut cache) = lyrics_cache().lock() {
        cache.put(cache_key.to_string(), value);
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
