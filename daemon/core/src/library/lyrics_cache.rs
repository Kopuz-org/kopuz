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

use db::CachedLyrics;
use utils::lyrics::Lyrics;

use super::LibraryService;

const NEGATIVE_TTL_SECS: i64 = 24 * 60 * 60;

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

/// What a stored entry answers: the words, a known absence, or (`None`) nothing still worth trusting.
fn answer(cached: CachedLyrics, now: i64) -> Option<Option<Lyrics>> {
    match cached {
        CachedLyrics::Found(lyrics) => Some(Some(lyrics)),
        // An expired miss reads as "nothing stored", so the providers run again.
        CachedLyrics::Missing { at } => {
            (now.saturating_sub(at) < NEGATIVE_TTL_SECS).then_some(None)
        }
    }
}

impl LibraryService {
    pub(super) async fn persisted_lyrics(&self, cache_key: &str) -> Option<Option<Lyrics>> {
        let cached = self.db.cached_lyrics(cache_key).await.ok()??;
        answer(cached, now_unix())
    }

    pub(super) async fn persist_lyrics(&self, cache_key: &str, value: &Option<Lyrics>) {
        if let Err(error) = self.db.cache_lyrics(cache_key, value.as_ref()).await {
            tracing::debug!(%error, "storing lyrics failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_miss_reads_back_as_a_known_absence() {
        let now = now_unix();
        assert_eq!(answer(CachedLyrics::Missing { at: now }, now), Some(None));
    }

    #[test]
    fn an_expired_miss_reads_as_nothing_stored_so_the_providers_run_again() {
        let now = now_unix();
        let stale = now - NEGATIVE_TTL_SECS - 1;
        assert_eq!(answer(CachedLyrics::Missing { at: stale }, now), None);
    }

    #[test]
    fn found_words_are_the_answer() {
        let words = Lyrics::Plain("a line".into());
        assert_eq!(
            answer(CachedLyrics::Found(words.clone()), now_unix()),
            Some(Some(words))
        );
    }
}
