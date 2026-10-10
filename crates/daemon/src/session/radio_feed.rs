//! A radio queue that tops itself up as it plays: once few tracks follow the
//! current one, the next page is fetched from the source that started it and
//! appended.

use std::collections::HashSet;

use super::*;

/// Radio asks for more once fewer than this many tracks follow the current one.
pub(super) const RADIO_LOW_WATER: usize = 5;

pub(super) struct RadioFeed {
    /// Tells a page that lands after its feed was replaced from one for this feed.
    id: u64,
    /// What to ask the source for next.
    cursor: String,
    /// Every cursor this feed has asked with. A source handing one back again
    /// would page the same batch forever.
    spent: HashSet<String>,
    fetch: Option<JoinHandle<()>>,
    /// The queue ran out while a page was on its way; play on once it lands.
    resume: bool,
}

impl Drop for RadioFeed {
    fn drop(&mut self) {
        if let Some(fetch) = self.fetch.take() {
            fetch.abort();
        }
    }
}

/// The tracks of `page` the queue does not already hold, by key, in the order
/// the source gave them.
pub(super) fn fresh_tracks(queue: &[Track], page: Vec<Track>) -> Vec<Track> {
    let mut seen: HashSet<String> = queue
        .iter()
        .map(|track| track.id.key().to_string())
        .collect();
    page.into_iter()
        .filter(|track| seen.insert(track.id.key().to_string()))
        .collect()
}

impl Session {
    /// Start a new feed for the queue just put in place, or end radio for one
    /// that is not a radio. A fetch for the old queue is abandoned either way.
    pub(super) fn set_radio_feed(&mut self, cursor: Option<String>) {
        self.radio_feed = cursor.map(|cursor| {
            self.next_feed_id += 1;
            RadioFeed {
                id: self.next_feed_id,
                cursor,
                spent: HashSet::new(),
                fetch: None,
                resume: false,
            }
        });
    }

    pub(super) fn radio_cursor(&self) -> Option<String> {
        self.radio_feed.as_ref().map(|feed| feed.cursor.clone())
    }

    /// The queue ran out under a radio: play on once the page lands.
    pub(super) fn resume_radio_when_extended(&mut self) {
        if let Some(feed) = self.radio_feed.as_mut() {
            feed.resume = true;
        }
        self.top_up_radio();
    }

    pub(super) fn top_up_radio(&mut self) {
        let len = self.model.len();
        let remaining = len.saturating_sub(self.model.current_position() + 1);
        let Some(feed) = self.radio_feed.as_mut() else {
            return;
        };
        if feed.fetch.is_some() || len == 0 || remaining >= RADIO_LOW_WATER {
            return;
        }
        let (id, cursor) = (feed.id, feed.cursor.clone());
        let materializer = self.materializer.clone();
        let cmd_tx = self.cmd_tx.clone();
        feed.fetch = Some(tokio::spawn(async move {
            let result =
                tokio::time::timeout(MATERIALIZE_TIMEOUT, materializer.more_radio(&cursor))
                    .await
                    .unwrap_or_else(|_| {
                        Err(ApiError::new(
                            api::ErrorCode::SourceUnreachable,
                            "radio top-up timed out",
                        ))
                    });
            let _ = cmd_tx.send(SessionCmd::RadioTopUp {
                feed: id,
                cursor,
                result: Box::new(result),
            });
        }));
    }

    pub(super) fn apply_radio_page(
        &mut self,
        feed_id: u64,
        cursor: String,
        result: Result<RadioPage, ApiError>,
        state_tx: &watch::Sender<PlayerState>,
    ) {
        let Some(feed) = self.radio_feed.as_mut().filter(|feed| feed.id == feed_id) else {
            return;
        };
        feed.fetch = None;
        let page = match result {
            Ok(page) => page,
            Err(error) => {
                tracing::warn!(%error, "radio top-up failed; the radio ends here");
                self.set_radio_feed(None);
                self.queue_dirty = true;
                return;
            }
        };
        feed.spent.insert(cursor);
        let fresh = fresh_tracks(self.model.items(), page.tracks);
        let next = page.more.filter(|more| !feed.spent.contains(more));
        let resume = std::mem::take(&mut feed.resume);
        tracing::debug!(
            added = fresh.len(),
            more = next.is_some(),
            "radio topped up the queue"
        );
        // A page with nothing new is a source going round in circles.
        match (next, fresh.is_empty()) {
            (Some(next), false) => feed.cursor = next,
            _ => self.set_radio_feed(None),
        }
        if fresh.is_empty() {
            self.queue_dirty = true;
            return;
        }
        self.model.add(fresh);
        if resume
            && self.intent == PlaybackIntent::Stopped
            && let Err(error) = self.play_next(false, state_tx)
        {
            tracing::warn!(%error, "radio could not play on after topping up");
        }
        self.publish(state_tx, true);
    }
}

#[cfg(test)]
mod tests {
    use super::fresh_tracks;

    fn track(key: &str) -> reader::Track {
        reader::Track {
            id: reader::TrackId::Local(std::path::PathBuf::from(key)),
            cover: None,
            album_id: String::new(),
            title: key.to_string(),
            artist: String::new(),
            album: String::new(),
            duration: 60,
            khz: 0,
            bitrate: 0,
            track_number: None,
            disc_number: None,
            musicbrainz_release_id: None,
            musicbrainz_recording_id: None,
            musicbrainz_track_id: None,
            playlist_item_id: None,
            credits: Vec::new(),
            artists: Vec::new(),
            replay_gain: config::ReplayGainInfo::default(),
        }
    }

    #[test]
    fn a_page_loses_what_the_queue_holds_and_its_own_repeats() {
        let queue = [track("a"), track("b")];
        let page = vec![track("b"), track("c"), track("a"), track("d"), track("c")];

        let fresh = fresh_tracks(&queue, page);

        assert_eq!(
            fresh.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(),
            ["c", "d"]
        );
    }
}
