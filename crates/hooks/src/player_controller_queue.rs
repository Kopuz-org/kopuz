use config::BackBehavior;
use dioxus::logger::tracing;
use dioxus::prelude::*;
use reader::Track;

use crate::playback_ref::PlaybackItemRef;
use crate::use_player_controller::{LoopMode, PlayerController};

impl PlayerController {
    pub fn play_next(&mut self) {
        self.play_next_with_transition(false);
    }

    pub fn play_next_with_crossfade(&mut self) {
        self.play_next_with_transition(true);
    }

    fn play_next_with_transition(&mut self, allow_crossfade: bool) {
        if *self.is_loading.peek() {
            self.skip_in_progress.set(false);
        }

        let idx = *self.current_queue_index.peek();
        let queue_len = self.queue.peek().len();

        if queue_len == 0 {
            self.skip_in_progress.set(false);
            return;
        }

        let loop_mode = *self.loop_mode.peek();

        match loop_mode {
            LoopMode::Track => {
                self.play_track_with_history(idx, allow_crossfade);
            }
            _ => {
                if idx + 1 >= queue_len && loop_mode == LoopMode::None {
                    // End of queue. Fully reset the transition state — otherwise a
                    // crossfade that fired just before this skip left a resolve in
                    // flight (with its index commit deferred): bumping the
                    // generation invalidates that stale resolve so it can't restart
                    // playback, and clearing is_loading keeps the player from
                    // freezing (every auto-advance gates on !is_loading, so a stuck
                    // is_loading can't be recovered by pause/resume).
                    self.play_generation.with_mut(|g| *g += 1);
                    self.clear_pending_crossfade_ui();
                    self.is_loading.set(false);
                    self.skip_in_progress.set(false);
                    self.player.write().pause();
                    self.is_playing.set(false);
                    return;
                }
                let next_idx = if idx + 1 < queue_len { idx + 1 } else { 0 };
                self.play_track_with_history(next_idx, allow_crossfade);
            }
        }
    }

    fn play_track_with_history(&mut self, track_idx: usize, allow_crossfade: bool) {
        let current_idx = *self.current_queue_index.peek();
        self.history.with_mut(|h| {
            if h.last() != Some(&current_idx) {
                h.push(current_idx);
            }
        });
        self.play_track_no_history_with_transition(track_idx, allow_crossfade);
    }

    pub fn play_prev(&mut self) {
        let progress = *self.current_song_progress.peek();
        let back_behavior = self.config.peek().back_behavior;

        if back_behavior == BackBehavior::RewindThenPrev && progress > 3 {
            self.seek(std::time::Duration::ZERO);
            return;
        }

        let idx = *self.current_queue_index.peek();
        let queue_len = self.queue.peek().len();

        if queue_len == 0 {
            return;
        }

        if let Some(prev_idx) = self.history.with_mut(|h| h.pop()) {
            self.play_track_no_history_without_crossfade(prev_idx);
            return;
        }

        if idx > 0 {
            self.play_track_no_history_without_crossfade(idx - 1);
        } else if *self.loop_mode.peek() == LoopMode::Queue {
            self.play_track_no_history_without_crossfade(queue_len - 1);
        }
    }

    /// Rebuilds `shuffle_order` as a full permutation of the queue: the
    /// currently playing track at position 0, every other track after it
    /// shuffled as a single pool. Resets `current_queue_index` to 0, which
    /// acts as a pointer into `shuffle_order` while shuffle is on.
    pub(crate) fn rebuild_shuffle_order(&mut self) {
        use rand::seq::SliceRandom;
        let queue_len = self.queue.peek().len();
        let current_idx = *self.current_queue_index.peek();

        if queue_len == 0 {
            self.shuffle_order.set(Vec::new());
            self.current_queue_index.set(0);
            return;
        }

        // The current track keeps playing at position 0; every other track is
        // shuffled as a single pool so the ones before and after it mix freely
        // instead of playing as two separate groups (issue #362).
        let mut order: Vec<usize> = Vec::with_capacity(queue_len);
        order.push(current_idx);
        let mut rest: Vec<usize> = (0..queue_len).filter(|&i| i != current_idx).collect();
        rest.shuffle(&mut rand::rng());
        order.extend(rest);

        // reset current queue index to match the currently played track (now moved at pos 0)
        // will be used as a pointer to the retrieve the current track in the shuffled order
        self.current_queue_index.set(0);
        self.shuffle_order.set(order);
    }

    pub fn play_queue_shuffled(&mut self, tracks: Vec<Track>) {
        use rand::RngExt;
        let queue_len = tracks.len();
        if queue_len == 0 {
            return;
        }

        self.queue.set(tracks);
        let start = rand::rng().random_range(0..queue_len);
        self.play_track(start);
    }

    pub fn play_queue_linear(&mut self, tracks: Vec<Track>) {
        if tracks.is_empty() {
            return;
        }
        self.queue.set(tracks);
        // `play_track` already starts track 0 (with history + shuffle handling);
        // a second bare play call here just bumped the play generation and
        // spawned a duplicate stream resolve for the same video.
        self.play_track(0);
    }

    pub fn add_to_queue(&mut self, tracks: impl IntoIterator<Item = Track>) {
        let tracks: Vec<Track> = tracks.into_iter().collect();
        let count = tracks.len();
        if count == 0 {
            return;
        }

        self.queue.with_mut(|q| q.extend(tracks));

        if *self.shuffle.peek() {
            let q_len = self.queue.peek().len();
            let start_idx = q_len - count;
            self.shuffle_order.with_mut(|so| {
                (start_idx..q_len).for_each(|idx| so.push(idx));
            });
        }
    }

    pub fn queue_play_next(&mut self, tracks: impl IntoIterator<Item = Track>) {
        let tracks: Vec<Track> = tracks.into_iter().collect();
        let count = tracks.len();
        if count == 0 {
            return;
        }

        if *self.shuffle.peek() {
            let insert_at = if self.shuffle_order.peek().is_empty() {
                0
            } else {
                (*self.current_queue_index.peek() + 1).min(self.shuffle_order.peek().len())
            };
            let start_idx = self.queue.peek().len();
            self.queue.with_mut(|queue| queue.extend(tracks));
            self.shuffle_order.with_mut(|order| {
                for offset in 0..count {
                    order.insert(insert_at + offset, start_idx + offset);
                }
            });
            self.history.with_mut(|history| {
                Self::shift_indices_at_or_after(history, insert_at, count);
            });
        } else {
            let insert_at = if self.queue.peek().is_empty() {
                0
            } else {
                (*self.current_queue_index.peek() + 1).min(self.queue.peek().len())
            };
            self.queue.with_mut(|queue| {
                for (offset, track) in tracks.into_iter().enumerate() {
                    queue.insert(insert_at + offset, track);
                }
            });

            self.history.with_mut(|history| {
                Self::shift_indices_at_or_after(history, insert_at, count);
            });
        }
    }

    pub fn toggle_shuffle(&mut self) {
        let now_on = !*self.shuffle.peek();
        self.shuffle.set(now_on);
        if now_on {
            self.rebuild_shuffle_order();
        } else {
            // reset current queue index to match track index when turning off shuffle mode
            let current_idx = *self.current_queue_index.peek();
            self.current_queue_index.set(
                *self
                    .shuffle_order
                    .peek()
                    .get(current_idx)
                    .unwrap_or(&current_idx),
            );
        }
    }

    pub fn set_loop_mode(&mut self, mode: LoopMode) {
        self.loop_mode.set(mode);
    }
    pub(crate) fn cancel_radio_task(&mut self) {
        if let Some(task) = self.radio_task.take() {
            task.cancel();
        }
    }
    pub fn play_radio(&mut self, station_id: &str, stream_id: &str) {
        let path = format!("radio:{}:{}", station_id, stream_id);
        let track = Track {
            id: reader::models::TrackId::Local(std::path::PathBuf::from(path)),
            cover: None,
            album_id: "".to_string(),
            title: stream_id.to_string(),
            artist: station_id.to_string(),
            album: "Live Radio".to_string(),
            duration: u64::MAX,
            khz: 0,
            bitrate: 0,
            track_number: None,
            disc_number: None,
            musicbrainz_release_id: None,
            musicbrainz_recording_id: None,
            musicbrainz_track_id: None,
            playlist_item_id: None,
            artists: vec![],
        };

        let mut q = self.queue.write();
        q.clear();
        q.push(track);
        drop(q);

        self.shuffle_order.set(vec![0]);
        self.current_queue_index.set(0);
        self.history.write().clear();
        self.play_track_no_history_with_transition(0, false);
    }

    pub fn toggle_loop(&mut self) {
        self.loop_mode.with_mut(|l| *l = l.next());
    }

    /// Hard reset: stop playback, drop the queue/history/current track,
    /// clear any pending playback error. Called when the active server
    /// changes — without it, a queued `ytmusic:` track would be replayed
    /// through whichever backend's stream-builder is now active.
    pub fn reset_for_backend_switch(&mut self) {
        // Invalidate any in-flight track-load spawn (YT __YT_PENDING
        // resolver, Jellyfin stream open, etc.) so its eventual
        // completion doesn't start playback against the cleared queue
        // or post a stale error banner / clear is_loading for the new
        // backend's first track.
        self.play_generation.with_mut(|g| *g += 1);
        self.cancel_radio_task();
        self.player.write().stop_for_transition();
        self.is_playing.set(false);
        self.is_loading.set(false);
        self.skip_in_progress.set(false);
        self.queue.write().clear();
        self.history.write().clear();
        self.current_queue_index.set(0);
        self.clear_current_track_metadata();
        self.playback_error.set(None);
    }

    pub fn pause(&mut self) {
        let idx = *self.current_queue_index.peek();
        let is_radio = self
            .get_track_at(idx)
            .is_some_and(|t| PlaybackItemRef::parse(&t.id.uid()).is_radio());

        if is_radio {
            self.player.write().stop_for_transition();
        } else {
            self.player.write().pause();
        }
        self.is_playing.set(false);
    }

    pub fn resume(&mut self) {
        let idx = *self.current_queue_index.peek();
        let is_radio = self
            .get_track_at(idx)
            .is_some_and(|t| PlaybackItemRef::parse(&t.id.uid()).is_radio());

        if is_radio || !self.player.peek().can_resume() {
            if let Some(track) = self.get_track_at(idx) {
                if !is_radio {
                    let progress_secs = (*self.current_song_progress.peek()).min(track.duration);
                    self.set_pending_resume_for_track(&track, progress_secs);
                }
                self.play_track_no_history(idx);
            }
            return;
        }

        self.player.write().play_resume();
        self.is_playing.set(true);
    }

    pub fn toggle(&mut self) {
        if *self.is_playing.peek() {
            self.pause();
        } else {
            self.resume();
        }
    }

    // Swaps two queue items, taking into account shuffle state and current queue index
    pub fn swap_queue_item(&mut self, from: usize, to: usize) {
        let shuffle = *self.shuffle.peek();
        let len = if shuffle {
            self.shuffle_order.len()
        } else {
            self.queue.peek().len()
        };

        if from >= len || to >= len || from == to {
            return;
        }

        if shuffle {
            self.shuffle_order.with_mut(|so| so.swap(from, to));
        } else {
            self.queue.with_mut(|so| so.swap(from, to));
        }

        let current_idx = *self.current_queue_index.peek();
        if current_idx == from {
            self.current_queue_index.set(to);
        } else if current_idx == to {
            self.current_queue_index.set(from);
        }

        self.history.with_mut(|history| {
            for idx in history.iter_mut() {
                if *idx == from {
                    *idx = to;
                } else if *idx == to {
                    *idx = from;
                }
            }
        });
    }

    pub fn move_queue_item(&mut self, from: usize, to: usize) {
        let shuffle = *self.shuffle.peek();
        let len = if shuffle {
            self.shuffle_order.len()
        } else {
            self.queue.peek().len()
        };

        if from >= len || to >= len || from == to {
            return;
        }

        if shuffle {
            let idx = self.shuffle_order.remove(from);
            self.shuffle_order.insert(to, idx);

            let current_idx = *self.current_queue_index.peek();
            self.current_queue_index
                .set(Self::remap_queue_index(current_idx, from, to));

            self.history.with_mut(|history| {
                for idx in history.iter_mut() {
                    *idx = Self::remap_queue_index(*idx, from, to);
                }
            });
        } else {
            self.move_physical_queue_item(from, to);
        }
    }

    /// Does not check bounds. use `move_queue_item` instead.
    fn move_physical_queue_item(&mut self, from: usize, to: usize) {
        self.queue.with_mut(|queue| {
            let track = queue.remove(from);
            queue.insert(to, track);
        });

        let current_idx = *self.current_queue_index.peek();
        self.current_queue_index
            .set(Self::remap_queue_index(current_idx, from, to));

        self.history.with_mut(|history| {
            for idx in history.iter_mut() {
                *idx = Self::remap_queue_index(*idx, from, to);
            }
        });
    }

    pub fn restore_queue_state(
        &mut self,
        queue: Vec<Track>,
        current_queue_index: usize,
        progress_secs: u64,
        shuffle_order: Vec<usize>,
        shuffle_enabled: bool,
    ) {
        self.clear_pending_crossfade_ui();
        self.player.write().stop();
        self.is_playing.set(false);
        self.is_loading.set(false);
        self.skip_in_progress.set(false);
        self.history.set(Vec::new());
        self.queue.set(queue);
        self.shuffle.set(shuffle_enabled);
        self.shuffle_order.set(shuffle_order);

        let queue_len = self.queue.peek().len();
        if queue_len == 0 {
            self.current_queue_index.set(0);
            self.pending_resume.set(None);
            self.clear_current_track_metadata();
            return;
        }

        let idx = current_queue_index.min(queue_len - 1);
        let Some(track) = self.get_track_at(idx) else {
            self.current_queue_index.set(0);
            self.pending_resume.set(None);
            self.clear_current_track_metadata();
            tracing::warn!(
                "Could not find track at index {} while restoring queue state. (queue_len={})",
                idx,
                queue_len
            );
            return;
        };
        let progress_secs = progress_secs.min(track.duration);

        self.hydrate_current_track_metadata(idx, progress_secs);
        self.set_pending_resume_for_track(&track, progress_secs);
    }
}
