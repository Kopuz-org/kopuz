//! Pure queue/shuffle/loop state, ported from
//! `hooks/src/player_controller_queue.rs` with identical semantics.
//!
//! Positions are logical: while shuffle is on, `current` and every history
//! entry point into `shuffle_order` (a permutation of physical indices), and
//! only [`QueueModel::physical_index_of`] does the indirection. The model
//! decides what to play; committing that decision to the audio engine is the
//! session actor's job.

use api::LoopMode;
use reader::Track;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NextOutcome {
    /// Advance to this logical position.
    Play(usize),
    /// End of queue under the current loop mode: stop, don't wrap.
    EndOfQueue,
    Empty,
}

#[derive(Debug, Clone, Default)]
pub struct QueueModel {
    items: Vec<Track>,
    history: Vec<usize>,
    shuffle: bool,
    shuffle_order: Vec<usize>,
    current: usize,
    loop_mode: LoopMode,
}

impl QueueModel {
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn items(&self) -> &[Track] {
        &self.items
    }

    pub fn current_position(&self) -> usize {
        self.current
    }

    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    pub fn shuffle_order(&self) -> &[usize] {
        &self.shuffle_order
    }

    pub fn history(&self) -> &[usize] {
        &self.history
    }

    pub fn loop_mode(&self) -> LoopMode {
        self.loop_mode
    }

    pub fn set_loop_mode(&mut self, mode: LoopMode) {
        self.loop_mode = mode;
    }

    pub fn toggle_loop(&mut self) {
        self.loop_mode = self.loop_mode.next();
    }

    /// Physical queue index for a logical position. Deliberately unbounded in
    /// linear mode, mirroring the hooks version: `track_at` does the bounds
    /// check against the queue itself.
    pub fn physical_index_of(&self, position: usize) -> Option<usize> {
        if self.shuffle {
            self.shuffle_order.get(position).copied()
        } else {
            Some(position)
        }
    }

    pub fn track_at(&self, position: usize) -> Option<&Track> {
        let idx = self.physical_index_of(position)?;
        self.items.get(idx)
    }

    pub(crate) fn track_at_mut(&mut self, position: usize) -> Option<&mut Track> {
        let idx = self.physical_index_of(position)?;
        self.items.get_mut(idx)
    }

    pub fn current_track(&self) -> Option<&Track> {
        self.track_at(self.current)
    }

    /// Whether advancing from `idx` lands on another track. The crossfade arm
    /// and the actual advance MUST agree on this: if the arm fires when the
    /// advance would hit end-of-queue, playback pauses mid-fade and the song
    /// dies ~crossfade-seconds early. Shuffle-agnostic on purpose.
    pub fn has_following_track(idx: usize, queue_len: usize, loop_mode: LoopMode) -> bool {
        if queue_len == 0 {
            return false;
        }
        match loop_mode {
            LoopMode::Track | LoopMode::Queue => true,
            LoopMode::None => idx + 1 < queue_len,
        }
    }

    pub fn has_next_track(&self) -> bool {
        Self::has_following_track(self.current, self.items.len(), self.loop_mode)
    }

    fn push_history_dedup(&mut self) {
        if self.history.last() != Some(&self.current) {
            self.history.push(self.current);
        }
    }

    /// The Next decision: repairs the shuffle permutation first so end-of-queue
    /// is measured against a permutation that still covers the queue. On
    /// `Play`, history is pushed and `current` moves.
    pub fn advance_next(&mut self) -> NextOutcome {
        let idx = self.current;
        let queue_len = if self.shuffle {
            self.repair_shuffle_order();
            self.shuffle_order.len()
        } else {
            self.items.len()
        };

        if queue_len == 0 {
            return NextOutcome::Empty;
        }

        match self.loop_mode {
            LoopMode::Track => {
                self.push_history_dedup();
                NextOutcome::Play(idx)
            }
            _ => {
                if !Self::has_following_track(idx, queue_len, self.loop_mode) {
                    return NextOutcome::EndOfQueue;
                }
                let next_idx = if idx + 1 < queue_len { idx + 1 } else { 0 };
                self.push_history_dedup();
                self.current = next_idx;
                NextOutcome::Play(next_idx)
            }
        }
    }

    /// The Previous decision, minus the caller-owned gates (crossfade revert,
    /// rewind-then-prev). Pops history first; falls back to `position - 1`,
    /// then to the queue tail under `LoopMode::Queue`.
    pub fn previous_position(&mut self) -> Option<usize> {
        let idx = self.current;
        let queue_len = if self.shuffle {
            self.repair_shuffle_order();
            self.shuffle_order.len()
        } else {
            self.items.len()
        };

        if queue_len == 0 {
            return None;
        }

        if let Some(prev_idx) = self.history.pop() {
            self.current = prev_idx;
            return Some(prev_idx);
        }

        if idx > 0 {
            self.current = idx - 1;
            Some(idx - 1)
        } else if self.loop_mode == LoopMode::Queue {
            self.current = queue_len - 1;
            Some(queue_len - 1)
        } else {
            None
        }
    }

    /// Explicit jump to a physical index (a row click), with history. While
    /// shuffling, the target becomes position 0 of a fresh permutation, which
    /// nets out to the old enable/disable workaround in the hooks version.
    /// Returns the logical position to play.
    pub fn jump_to(&mut self, physical_idx: usize) -> usize {
        self.push_history_dedup();
        self.current = physical_idx;
        if self.shuffle {
            self.rebuild_shuffle_order();
        }
        self.current
    }

    /// Explicit jump to a logical play-order position without changing the
    /// running shuffle permutation.
    pub fn jump_to_position(&mut self, position: usize) -> Option<usize> {
        let queue_len = if self.shuffle {
            self.repair_shuffle_order();
            self.shuffle_order.len()
        } else {
            self.items.len()
        };
        if position >= queue_len {
            return None;
        }
        self.push_history_dedup();
        self.current = position;
        Some(position)
    }

    /// Replace the queue contents. Callers follow up with [`Self::jump_to`];
    /// history and permutation carry over exactly like the hooks version,
    /// where the jump's rebuild covers the shuffle case.
    pub fn replace(&mut self, tracks: Vec<Track>) {
        self.items = tracks;
    }

    pub fn add(&mut self, tracks: Vec<Track>) {
        let count = tracks.len();
        if count == 0 {
            return;
        }
        self.items.extend(tracks);
        if self.shuffle {
            let q_len = self.items.len();
            let start_idx = q_len - count;
            self.shuffle_order.extend(start_idx..q_len);
        }
    }

    /// Insert tracks immediately after the current position (in play order),
    /// shifting history entries so they keep pointing at the same tracks.
    pub fn insert_next(&mut self, tracks: Vec<Track>) {
        let count = tracks.len();
        if count == 0 {
            return;
        }

        if self.shuffle {
            let insert_at = if self.shuffle_order.is_empty() {
                0
            } else {
                (self.current + 1).min(self.shuffle_order.len())
            };
            let start_idx = self.items.len();
            self.items.extend(tracks);
            for offset in 0..count {
                self.shuffle_order
                    .insert(insert_at + offset, start_idx + offset);
            }
            Self::shift_indices_at_or_after(&mut self.history, insert_at, count);
        } else {
            let insert_at = if self.items.is_empty() {
                0
            } else {
                (self.current + 1).min(self.items.len())
            };
            for (offset, track) in tracks.into_iter().enumerate() {
                self.items.insert(insert_at + offset, track);
            }
            Self::shift_indices_at_or_after(&mut self.history, insert_at, count);
        }
    }

    /// Insert tracks at a play-order position (the queue view's drag-drop),
    /// keeping the current pointer and history aimed at the same tracks.
    pub fn insert_at(&mut self, position: usize, tracks: Vec<Track>) {
        let count = tracks.len();
        if count == 0 {
            return;
        }
        let was_empty = self.items.is_empty();
        if self.shuffle {
            self.repair_shuffle_order();
            let visual_insert = position.min(self.shuffle_order.len());
            let physical_insert = self
                .shuffle_order
                .get(visual_insert)
                .copied()
                .unwrap_or(self.items.len());
            for (offset, track) in tracks.into_iter().enumerate() {
                self.items.insert(physical_insert + offset, track);
            }
            for idx in &mut self.shuffle_order {
                if *idx >= physical_insert {
                    *idx += count;
                }
            }
            for offset in 0..count {
                self.shuffle_order
                    .insert(visual_insert + offset, physical_insert + offset);
            }
            if visual_insert <= self.current && !was_empty {
                self.current += count;
            }
            Self::shift_indices_at_or_after(&mut self.history, visual_insert, count);
        } else {
            let insert_at = position.min(self.items.len());
            for (offset, track) in tracks.into_iter().enumerate() {
                self.items.insert(insert_at + offset, track);
            }
            if insert_at <= self.current && !was_empty {
                self.current += count;
            }
            Self::shift_indices_at_or_after(&mut self.history, insert_at, count);
        }
    }

    pub fn set_shuffle(&mut self, on: bool) {
        if self.shuffle != on {
            self.toggle_shuffle();
        }
    }

    pub fn toggle_shuffle(&mut self) {
        self.shuffle = !self.shuffle;
        if self.shuffle {
            self.rebuild_shuffle_order();
        } else {
            self.current = self
                .shuffle_order
                .get(self.current)
                .copied()
                .unwrap_or(self.current);
        }
    }

    /// Rebuilds `shuffle_order` as a full permutation: the track at the
    /// physical index held in `current` stays at position 0, every other track
    /// shuffled as a single pool (issue #362), and `current` resets to 0 as
    /// the pointer into the permutation.
    pub fn rebuild_shuffle_order(&mut self) {
        use rand::seq::SliceRandom;
        let queue_len = self.items.len();
        let current_idx = self.current;

        if queue_len == 0 {
            self.shuffle_order = Vec::new();
            self.current = 0;
            return;
        }

        let mut order: Vec<usize> = Vec::with_capacity(queue_len);
        order.push(current_idx);
        let mut rest: Vec<usize> = (0..queue_len).filter(|&i| i != current_idx).collect();
        rest.shuffle(&mut rand::rng());
        order.extend(rest);

        self.current = 0;
        self.shuffle_order = order;
    }

    /// Position of a physical index inside the running permutation, or `None`
    /// when the permutation still doesn't cover it after repair.
    pub fn shuffle_position_of(&mut self, physical_idx: usize) -> Option<usize> {
        self.repair_shuffle_order();
        self.shuffle_order
            .iter()
            .position(|&idx| idx == physical_idx)
    }

    /// Make the permutation cover the whole queue again without disturbing the
    /// order it already has: missing entries are shuffled among themselves and
    /// appended rather than triggering a full reshuffle.
    pub fn repair_shuffle_order(&mut self) {
        let queue_len = self.items.len();
        if self.shuffle_order.len() == queue_len
            && self.shuffle_order.iter().all(|&idx| idx < queue_len)
            && self
                .shuffle_order
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                == queue_len
        {
            return;
        }
        self.shuffle_order = Self::repaired_order(&self.shuffle_order, queue_len);
    }

    pub fn repaired_order(order: &[usize], queue_len: usize) -> Vec<usize> {
        use rand::seq::SliceRandom;
        let mut seen = vec![false; queue_len];
        let mut repaired: Vec<usize> = Vec::with_capacity(queue_len);
        for &idx in order {
            if idx < queue_len && !seen[idx] {
                seen[idx] = true;
                repaired.push(idx);
            }
        }
        let mut missing: Vec<usize> = (0..queue_len).filter(|&idx| !seen[idx]).collect();
        missing.shuffle(&mut rand::rng());
        repaired.extend(missing);
        repaired
    }

    /// Remap a logical position after moving one entry from `from` to `to`.
    pub fn remap_queue_index(index: usize, from: usize, to: usize) -> usize {
        if index == from {
            to
        } else if from < to && index > from && index <= to {
            index - 1
        } else if to < from && index >= to && index < from {
            index + 1
        } else {
            index
        }
    }

    pub fn shift_indices_at_or_after(indices: &mut [usize], at: usize, by: usize) {
        for idx in indices {
            if *idx >= at {
                *idx += by;
            }
        }
    }

    pub fn swap(&mut self, from: usize, to: usize) {
        let len = if self.shuffle {
            self.shuffle_order.len()
        } else {
            self.items.len()
        };
        if from >= len || to >= len || from == to {
            return;
        }

        if self.shuffle {
            self.shuffle_order.swap(from, to);
        } else {
            self.items.swap(from, to);
        }

        if self.current == from {
            self.current = to;
        } else if self.current == to {
            self.current = from;
        }

        for idx in self.history.iter_mut() {
            if *idx == from {
                *idx = to;
            } else if *idx == to {
                *idx = from;
            }
        }
    }

    pub fn move_item(&mut self, from: usize, to: usize) {
        let len = if self.shuffle {
            self.shuffle_order.len()
        } else {
            self.items.len()
        };
        if from >= len || to >= len || from == to {
            return;
        }

        if self.shuffle {
            let idx = self.shuffle_order.remove(from);
            self.shuffle_order.insert(to, idx);
        } else {
            let track = self.items.remove(from);
            self.items.insert(to, track);
        }

        self.current = Self::remap_queue_index(self.current, from, to);
        for idx in self.history.iter_mut() {
            *idx = Self::remap_queue_index(*idx, from, to);
        }
    }

    /// Remove the entry at a logical position. Physical indices above the
    /// removed track shift down, so the permutation and history are remapped;
    /// history entries pointing at the removed position are dropped. `current`
    /// keeps its logical position (now the following track), clamped to the
    /// new tail.
    pub fn remove(&mut self, position: usize) -> Option<Track> {
        let physical = self.physical_index_of(position)?;
        if physical >= self.items.len() {
            return None;
        }
        let removed = self.items.remove(physical);

        if self.shuffle {
            if position < self.shuffle_order.len() {
                self.shuffle_order.remove(position);
            }
        } else {
            self.shuffle_order.retain(|&idx| idx != physical);
        }
        for idx in self.shuffle_order.iter_mut() {
            if *idx > physical {
                *idx -= 1;
            }
        }

        self.history.retain(|&idx| idx != position);
        for idx in self.history.iter_mut() {
            if *idx > position {
                *idx -= 1;
            }
        }

        if position < self.current {
            self.current -= 1;
        }
        let len = self.items.len();
        if len == 0 {
            self.current = 0;
        } else if self.current >= len {
            self.current = len - 1;
        }
        Some(removed)
    }

    /// Hard reset for a backend switch: queue, history, permutation, position.
    pub fn clear(&mut self) {
        self.items.clear();
        self.history.clear();
        self.shuffle_order.clear();
        self.current = 0;
    }

    /// Restore a persisted queue. Returns the clamped logical position whose
    /// track exists, or `None` when the queue is empty or the position cannot
    /// resolve (the caller then shows an empty player).
    pub fn restore(
        &mut self,
        items: Vec<Track>,
        current_position: usize,
        shuffle_order: Vec<usize>,
        shuffle_enabled: bool,
    ) -> Option<usize> {
        self.history = Vec::new();
        self.items = items;
        self.shuffle = shuffle_enabled;
        self.shuffle_order = shuffle_order;

        let queue_len = self.items.len();
        if queue_len == 0 {
            self.current = 0;
            return None;
        }

        if self.shuffle {
            self.repair_shuffle_order();
        }

        let idx = current_position.min(queue_len - 1);
        if self.track_at(idx).is_none() {
            self.current = 0;
            return None;
        }
        self.current = idx;
        Some(idx)
    }

    /// A play-order window for `GET /v1/queue`: `(logical position, track)`
    /// pairs. Repairs the permutation first so every position resolves.
    pub fn window(&mut self, offset: usize, limit: usize) -> Vec<(usize, Track)> {
        if self.shuffle {
            self.repair_shuffle_order();
        }
        let len = self.items.len();
        (offset..len.min(offset.saturating_add(limit)))
            .filter_map(|pos| self.track_at(pos).cloned().map(|t| (pos, t)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(n: usize) -> Track {
        Track {
            id: reader::models::TrackId::Local(std::path::PathBuf::from(format!("/t/{n}.mp3"))),
            cover: None,
            album_id: String::new(),
            title: format!("t{n}"),
            artist: String::new(),
            album: String::new(),
            duration: 100,
            khz: 0,
            bitrate: 0,
            track_number: None,
            disc_number: None,
            musicbrainz_release_id: None,
            musicbrainz_recording_id: None,
            musicbrainz_track_id: None,
            playlist_item_id: None,
            artists: vec![],
        }
    }

    fn model(n: usize) -> QueueModel {
        let mut m = QueueModel::default();
        m.replace((0..n).map(track).collect());
        m
    }

    #[test]
    fn has_following_track_matches_end_of_queue() {
        use LoopMode::{None, Queue, Track};

        assert!(!QueueModel::has_following_track(0, 0, None));
        assert!(!QueueModel::has_following_track(0, 0, Queue));

        assert!(!QueueModel::has_following_track(0, 1, None));
        assert!(!QueueModel::has_following_track(4, 5, None));

        assert!(QueueModel::has_following_track(0, 5, None));
        assert!(QueueModel::has_following_track(3, 5, None));

        assert!(QueueModel::has_following_track(0, 1, Queue));
        assert!(QueueModel::has_following_track(4, 5, Queue));
        assert!(QueueModel::has_following_track(4, 5, Track));
    }

    #[test]
    fn repaired_permutation_keeps_the_order_it_already_had() {
        let repaired = QueueModel::repaired_order(&[3, 0], 5);
        assert_eq!(&repaired[..2], &[3, 0]);
        assert_eq!(repaired.len(), 5);

        let mut covered = repaired.clone();
        covered.sort_unstable();
        assert_eq!(covered, vec![0, 1, 2, 3, 4]);

        let repaired = QueueModel::repaired_order(&[2, 9, 2, 0], 3);
        assert_eq!(&repaired[..2], &[2, 0]);
        assert_eq!(repaired.len(), 3);
    }

    #[test]
    fn insert_at_keeps_the_pointer_on_the_playing_track() {
        let mut m = model(3);
        m.jump_to(1);
        m.insert_at(0, vec![track(10), track(11)]);
        assert_eq!(m.len(), 5);
        assert_eq!(m.current_position(), 3);
        assert_eq!(
            m.current_track().map(|t| t.title.clone()),
            Some("t1".into())
        );
        assert_eq!(m.items()[0].title, "t10");
        assert_eq!(m.items()[1].title, "t11");

        let mut m = model(2);
        m.insert_at(99, vec![track(20)]);
        assert_eq!(m.items()[2].title, "t20");
        assert_eq!(m.current_position(), 0);
    }

    #[test]
    fn insert_at_while_shuffling_splices_the_permutation() {
        let mut m = model(4);
        m.toggle_shuffle();
        m.insert_at(2, vec![track(30), track(31)]);
        assert_eq!(m.len(), 6);
        assert_eq!(m.shuffle_order().len(), 6);
        assert_eq!(m.track_at(2).map(|t| t.title.clone()), Some("t30".into()));
        assert_eq!(m.track_at(3).map(|t| t.title.clone()), Some("t31".into()));
        let mut covered: Vec<usize> = m.shuffle_order().to_vec();
        covered.sort_unstable();
        assert_eq!(covered, vec![0, 1, 2, 3, 4, 5]);
        assert_eq!(
            m.current_track().map(|t| t.title.clone()),
            Some("t0".into())
        );
    }

    #[test]
    fn advance_stops_at_end_without_loop_and_wraps_with_queue_loop() {
        let mut m = model(3);
        assert_eq!(m.advance_next(), NextOutcome::Play(1));
        assert_eq!(m.advance_next(), NextOutcome::Play(2));
        assert_eq!(m.advance_next(), NextOutcome::EndOfQueue);
        assert_eq!(m.current_position(), 2);

        m.set_loop_mode(LoopMode::Queue);
        assert_eq!(m.advance_next(), NextOutcome::Play(0));
    }

    #[test]
    fn track_loop_replays_current_position() {
        let mut m = model(3);
        m.set_loop_mode(LoopMode::Track);
        assert_eq!(m.advance_next(), NextOutcome::Play(0));
        assert_eq!(m.current_position(), 0);
    }

    #[test]
    fn previous_pops_history_before_falling_back() {
        let mut m = model(4);
        m.jump_to(2);
        assert_eq!(m.history(), &[0]);
        assert_eq!(m.previous_position(), Some(0));
        assert_eq!(m.previous_position(), None);

        m.set_loop_mode(LoopMode::Queue);
        assert_eq!(m.previous_position(), Some(3));
    }

    #[test]
    fn shuffle_pins_current_track_and_collapse_restores_physical_index() {
        let mut m = model(5);
        m.jump_to(3);
        m.toggle_shuffle();
        assert!(m.shuffle());
        assert_eq!(m.current_position(), 0);
        assert_eq!(m.shuffle_order()[0], 3);
        assert_eq!(
            m.current_track().map(|t| t.title.clone()),
            Some("t3".into())
        );

        let mut covered = m.shuffle_order().to_vec();
        covered.sort_unstable();
        assert_eq!(covered, vec![0, 1, 2, 3, 4]);

        m.toggle_shuffle();
        assert!(!m.shuffle());
        assert_eq!(m.current_position(), 3);
        assert_eq!(
            m.current_track().map(|t| t.title.clone()),
            Some("t3".into())
        );
    }

    #[test]
    fn logical_jump_preserves_shuffle_order() {
        let mut m = model(5);
        m.toggle_shuffle();
        let order = m.shuffle_order().to_vec();
        let target = m.track_at(2).map(|track| track.title.clone());

        assert_eq!(m.jump_to_position(2), Some(2));
        assert_eq!(m.current_position(), 2);
        assert_eq!(m.current_track().map(|track| track.title.clone()), target);
        assert_eq!(m.shuffle_order(), order);
    }

    #[test]
    fn insert_next_lands_after_current_and_shifts_history() {
        let mut m = model(3);
        m.jump_to(1);
        m.insert_next(vec![track(10), track(11)]);
        assert_eq!(m.len(), 5);
        assert_eq!(m.items()[2].title, "t10");
        assert_eq!(m.items()[3].title, "t11");
        assert_eq!(m.history(), &[0]);
        assert_eq!(
            m.current_track().map(|t| t.title.clone()),
            Some("t1".into())
        );

        let mut m = model(3);
        m.jump_to(0);
        m.jump_to(2);
        assert_eq!(m.history(), &[0]);
        m.insert_next(vec![track(10)]);
        assert_eq!(m.history(), &[0]);
        assert_eq!(m.items()[3].title, "t10");
    }

    #[test]
    fn insert_next_while_shuffling_splices_the_permutation() {
        let mut m = model(4);
        m.toggle_shuffle();
        let before = m.shuffle_order().to_vec();
        m.insert_next(vec![track(10), track(11)]);
        assert_eq!(m.len(), 6);
        assert_eq!(m.shuffle_order().len(), 6);
        assert_eq!(m.shuffle_order()[1], 4);
        assert_eq!(m.shuffle_order()[2], 5);
        assert_eq!(m.shuffle_order()[0], before[0]);
        assert_eq!(m.track_at(1).map(|t| t.title.clone()), Some("t10".into()));
        assert_eq!(m.track_at(2).map(|t| t.title.clone()), Some("t11".into()));
    }

    #[test]
    fn move_item_remaps_current_and_history() {
        let mut m = model(5);
        m.jump_to(1);
        m.jump_to(3);
        assert_eq!(m.history(), &[0, 1]);

        m.move_item(3, 0);
        assert_eq!(m.current_position(), 0);
        assert_eq!(
            m.current_track().map(|t| t.title.clone()),
            Some("t3".into())
        );
        assert_eq!(m.history(), &[1, 2]);

        assert_eq!(QueueModel::remap_queue_index(2, 0, 4), 1);
        assert_eq!(QueueModel::remap_queue_index(0, 0, 4), 4);
        assert_eq!(QueueModel::remap_queue_index(5, 0, 4), 5);
    }

    #[test]
    fn swap_follows_current_and_history() {
        let mut m = model(4);
        m.jump_to(2);
        m.swap(2, 0);
        assert_eq!(m.current_position(), 0);
        assert_eq!(
            m.current_track().map(|t| t.title.clone()),
            Some("t2".into())
        );
        assert_eq!(m.history(), &[2]);
    }

    #[test]
    fn remove_remaps_current_history_and_permutation() {
        let mut m = model(5);
        m.jump_to(1);
        m.jump_to(3);
        assert_eq!(m.history(), &[0, 1]);

        let removed = m.remove(1).expect("removed");
        assert_eq!(removed.title, "t1");
        assert_eq!(m.len(), 4);
        assert_eq!(m.current_position(), 2);
        assert_eq!(
            m.current_track().map(|t| t.title.clone()),
            Some("t3".into())
        );
        assert_eq!(m.history(), &[0]);

        let removed = m.remove(3);
        assert!(removed.is_none() || m.len() == 3);
    }

    #[test]
    fn remove_last_position_clamps_current() {
        let mut m = model(2);
        m.jump_to(1);
        m.remove(1).expect("removed");
        assert_eq!(m.current_position(), 0);
        assert_eq!(
            m.current_track().map(|t| t.title.clone()),
            Some("t0".into())
        );

        m.remove(0).expect("removed");
        assert!(m.is_empty());
        assert_eq!(m.current_position(), 0);
        assert!(m.current_track().is_none());
    }

    #[test]
    fn remove_while_shuffling_keeps_a_valid_permutation() {
        let mut m = model(5);
        m.toggle_shuffle();
        let victim_position = 2;
        let victim_title = m.track_at(victim_position).map(|t| t.title.clone());
        m.remove(victim_position).expect("removed");
        assert_eq!(m.len(), 4);
        assert_eq!(m.shuffle_order().len(), 4);
        let mut covered = m.shuffle_order().to_vec();
        covered.sort_unstable();
        assert_eq!(covered, vec![0, 1, 2, 3]);
        for pos in 0..4 {
            let title = m.track_at(pos).map(|t| t.title.clone());
            assert!(title.is_some());
            assert_ne!(title, victim_title);
        }
    }

    #[test]
    fn restore_clamps_out_of_range_positions() {
        let mut m = QueueModel::default();
        let restored = m.restore((0..3).map(track).collect(), 9, vec![], false);
        assert_eq!(restored, Some(2));
        assert_eq!(m.current_position(), 2);

        let mut m = QueueModel::default();
        assert_eq!(m.restore(Vec::new(), 3, vec![], true), None);
        assert_eq!(m.current_position(), 0);
    }

    #[test]
    fn restore_repairs_invalid_shuffle_order() {
        let mut m = QueueModel::default();
        assert_eq!(
            m.restore((0..3).map(track).collect(), 1, vec![8, 9, 10], true),
            Some(1)
        );
        let mut order = m.shuffle_order().to_vec();
        order.sort_unstable();
        assert_eq!(order, vec![0, 1, 2]);
        assert!(m.current_track().is_some());
    }

    #[test]
    fn window_lists_play_order_and_repairs_stale_permutations() {
        let mut m = model(4);
        let win = m.window(1, 2);
        assert_eq!(win.len(), 2);
        assert_eq!(win[0].0, 1);
        assert_eq!(win[0].1.title, "t1");

        let mut m = model(4);
        m.toggle_shuffle();
        m.add(vec![track(10)]);
        let win = m.window(0, 10);
        assert_eq!(win.len(), 5);
        let mut physical: Vec<usize> = m.shuffle_order().to_vec();
        physical.sort_unstable();
        assert_eq!(physical, vec![0, 1, 2, 3, 4]);
    }
}
