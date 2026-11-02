use api::{ApiEvent, QueueEdit, QueueSnapshot, TrackInfo};
use gpui_kit::{App, Context, Entity, Subscription, Task};
use link::Link;

/// What the queue adds up to: "3/57 • 3:41:02" without folding every row per frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QueueSummary {
    /// One-based place of the current row; zero when nothing is current.
    pub position: usize,
    pub count: usize,
    /// What the known durations add up to; `None` when no row has one.
    pub total_ms: Option<u64>,
}

/// The whole queue in play order, refetched whenever its revision moves.
pub struct QueueModel {
    snapshot: QueueSnapshot,
    summary: QueueSummary,
    generation: u64,
    _fetch: Option<Task<()>>,
    _subscription: Subscription,
}

impl QueueModel {
    pub fn global(cx: &App) -> Entity<Self> {
        crate::handle(cx)
    }

    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let connection = Link::connection(cx);
        let subscription = cx.subscribe(&connection, |this, _, event: &ApiEvent, cx| match event {
            ApiEvent::QueueChanged { rev, .. } if *rev != this.snapshot.rev => this.refetch(cx),
            // The rows are still right, only the playhead moved: take it without a round trip.
            ApiEvent::PlayerState(state)
                if state.queue.rev == this.snapshot.rev
                    && state.queue.index != this.snapshot.position =>
            {
                this.snapshot.position = state.queue.index;
                this.resummarize();
                crate::notify(cx);
            }
            ApiEvent::Resync => this.refetch(cx),
            _ => {}
        });
        Self {
            snapshot: QueueSnapshot::default(),
            summary: QueueSummary::default(),
            generation: 0,
            _fetch: None,
            _subscription: subscription,
        }
    }

    /// Recomputed where the queue changes, which is rarely; reading it is what happens per frame.
    fn resummarize(&mut self) {
        let total_ms = api::total_duration_secs(&self.snapshot.items).map(|secs| secs * 1000);
        self.summary = QueueSummary {
            position: self.snapshot.position.map_or(0, |at| at as usize + 1),
            count: self.snapshot.items.len(),
            total_ms,
        };
    }

    pub fn summary(&self) -> QueueSummary {
        self.summary
    }

    fn refetch(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        let call = Link::global(cx).call(|api| async move { api.queue_snapshot().await });
        self._fetch = Some(cx.spawn(async move |this, cx| {
            let Ok(snapshot) = call.await else { return };
            this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.snapshot = snapshot;
                    this.resummarize();
                    crate::notify(cx);
                }
            })
            .ok();
        }));
    }

    /// Rows in play order: index 0 plays first.
    pub fn items(&self) -> &[TrackInfo] {
        &self.snapshot.items
    }

    /// Play-order index of the current row.
    pub fn position(&self) -> Option<u32> {
        self.snapshot.position
    }

    pub fn current(&self) -> Option<&TrackInfo> {
        self.snapshot
            .position
            .and_then(|position| self.snapshot.items.get(position as usize))
    }

    pub fn len(&self) -> usize {
        self.snapshot.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.snapshot.items.is_empty()
    }

    pub fn shuffle(&self) -> bool {
        self.snapshot.shuffle
    }

    pub fn rev(&self) -> u64 {
        self.snapshot.rev
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.snapshot.items.iter().any(|item| item.key == key)
    }

    pub fn jump(&mut self, index: u32, cx: &mut Context<Self>) {
        self.snapshot.position = Some(index);
        self.resummarize();
        crate::notify(cx);
        edit(QueueEdit::Jump { index }, cx);
    }

    /// Move a row, locally first; a daemon that refuses puts the queue back where it was.
    pub fn move_item(&mut self, from: u32, to: u32, cx: &mut Context<Self>) {
        let count = self.snapshot.items.len() as u32;
        if from >= count || to >= count || from == to {
            return;
        }
        let before = self.snapshot.clone();
        let item = self.snapshot.items.remove(from as usize);
        self.snapshot.items.insert(to as usize, item);
        self.snapshot.position = self.snapshot.position.map(|at| reindex(at, from, to));
        self.resummarize();
        crate::notify(cx);
        self.commit(QueueEdit::Move { from, to }, before, cx);
    }

    /// Drop a row, locally first; the removed track comes back so a caller can offer an undo.
    pub fn remove(&mut self, index: u32, cx: &mut Context<Self>) -> Option<TrackInfo> {
        let gone = self.snapshot.items.get(index as usize)?.clone();
        let before = self.snapshot.clone();
        self.snapshot.items.remove(index as usize);
        self.snapshot.position = self.snapshot.position.and_then(|at| match at.cmp(&index) {
            std::cmp::Ordering::Greater => Some(at - 1),
            std::cmp::Ordering::Equal => {
                Some(at).filter(|at| (*at as usize) < self.snapshot.items.len())
            }
            std::cmp::Ordering::Less => Some(at),
        });
        self.resummarize();
        crate::notify(cx);
        self.commit(QueueEdit::Remove { index }, before, cx);
        Some(gone)
    }

    pub fn insert(&mut self, index: u32, keys: Vec<String>, cx: &mut Context<Self>) {
        edit(QueueEdit::Insert { index, keys }, cx);
    }

    /// The rows after the one playing, in play order.
    pub fn upcoming(&self) -> &[TrackInfo] {
        let from = self.snapshot.position.map_or(0, |at| at as usize + 1);
        self.snapshot.items.get(from..).unwrap_or_default()
    }

    /// Drop everything after the current row, locally first, in one daemon edit.
    pub fn clear_upcoming(&mut self, cx: &mut Context<Self>) {
        if self.upcoming().is_empty() {
            return;
        }
        let before = self.snapshot.clone();
        let first = self.snapshot.position.map_or(0, |at| at as usize + 1);
        self.snapshot.items.truncate(first);
        self.resummarize();
        crate::notify(cx);
        self.commit(QueueEdit::ClearUpcoming, before, cx);
    }

    /// Replace everything after the current row with `keys`, in one daemon edit; empty clears it.
    pub fn replace_upcoming(&mut self, keys: Vec<String>, cx: &mut Context<Self>) {
        if keys.is_empty() {
            self.clear_upcoming(cx);
            return;
        }
        // Rows not already held arrive with the daemon's snapshot; the ones held move now.
        let before = self.snapshot.clone();
        let first = self.snapshot.position.map_or(0, |at| at as usize + 1);
        let held: Vec<TrackInfo> = self.snapshot.items.drain(first.min(self.len())..).collect();
        for key in &keys {
            if let Some(track) = held.iter().find(|track| &track.key == key) {
                self.snapshot.items.push(track.clone());
            }
        }
        self.resummarize();
        crate::notify(cx);
        self.commit(QueueEdit::ReplaceUpcoming { keys }, before, cx);
    }

    /// Randomise what plays after the current row; the daemon draws the order.
    pub fn shuffle_upcoming(&mut self, cx: &mut Context<Self>) {
        if self.upcoming().len() < 2 {
            return;
        }
        let before = self.snapshot.clone();
        self.commit(QueueEdit::ShuffleUpcoming, before, cx);
    }

    /// Send an edit the local snapshot has already taken, restoring it if the daemon refuses.
    fn commit(&mut self, change: QueueEdit, before: QueueSnapshot, cx: &mut Context<Self>) {
        let generation = self.generation;
        let call = Link::global(cx).call(move |api| async move { api.queue_edit(change).await });
        cx.spawn(async move |this, cx| {
            let Err(error) = call.await else { return };
            this.update(cx, |this, cx| this.roll_back(generation, before, cx))
                .ok();
            cx.update(|cx| crate::Notices::failed(&error, cx));
        })
        .detach();
    }

    /// Put the snapshot back, unless a fresher one has landed since the edit went out.
    fn roll_back(&mut self, generation: u64, before: QueueSnapshot, cx: &mut Context<Self>) {
        if self.generation != generation {
            return;
        }
        self.snapshot = before;
        self.resummarize();
        crate::notify(cx);
    }
}

/// Where the current row lands once the row at `from` has been moved to `to`.
fn reindex(at: u32, from: u32, to: u32) -> u32 {
    if at == from {
        to
    } else if from < at && at <= to {
        at - 1
    } else if to <= at && at < from {
        at + 1
    } else {
        at
    }
}

fn edit(edit: QueueEdit, cx: &App) {
    Link::global(cx).fire(cx, "queue edit", move |api| async move {
        api.queue_edit(edit).await
    });
}

#[cfg(test)]
mod tests {
    use super::reindex;

    #[test]
    fn the_moved_row_carries_the_playhead_with_it() {
        assert_eq!(reindex(3, 3, 7), 7);
    }

    #[test]
    fn a_row_moved_past_the_playhead_shifts_it_back() {
        assert_eq!(reindex(4, 2, 6), 3);
    }

    #[test]
    fn a_row_moved_in_front_of_the_playhead_shifts_it_on() {
        assert_eq!(reindex(4, 8, 2), 5);
    }

    #[test]
    fn a_move_on_the_far_side_leaves_the_playhead_alone() {
        assert_eq!(reindex(2, 5, 8), 2);
    }
}
