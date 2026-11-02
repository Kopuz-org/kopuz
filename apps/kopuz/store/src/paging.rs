//! Reading a long listing a screenful at a time, instead of asking for the whole library.

use std::ops::Range;

use crate::api::Page;

/// How many rows one read asks for; a window snaps to a multiple of this, so scrolling inside one costs nothing.
pub const CHUNK: usize = 100;

/// Rows kept either side of the viewport, so a slow scroll does not stall at the chunk edge.
pub const BUFFER: usize = 25;

/// One `Page{offset, limit}`, spelled the way a call site reads.
pub fn page(offset: usize, limit: usize) -> Page {
    Page {
        offset: offset as u32,
        limit: limit.max(1) as u32,
    }
}

/// Which slice of a listing is held, how long the listing is, and what read is in flight.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pager {
    total: usize,
    offset: usize,
    held: usize,
    requested: Option<Page>,
}

impl Pager {
    pub fn new() -> Self {
        Self::default()
    }

    /// How many rows the filtered listing holds, which is what a count or a scrollbar wants.
    pub fn total(&self) -> usize {
        self.total
    }

    /// Where the held slice starts in the whole listing.
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Whether `index` is inside the slice that is actually loaded.
    pub fn holds(&self, index: usize) -> bool {
        index >= self.offset && index < self.offset + self.held
    }

    /// Take what a read answered: where it starts, how many rows it brought, and the whole length.
    pub fn adopt(&mut self, offset: usize, held: usize, total: usize) {
        self.offset = offset;
        self.held = held;
        self.total = total;
    }

    /// Forget everything, for a filter or sort change that makes the held rows the wrong ones.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// The page covering `range` plus its buffer, snapped out to whole chunks.
    pub fn page_for(&self, range: &Range<usize>) -> Page {
        let start = range.start.saturating_sub(BUFFER) / CHUNK * CHUNK;
        let reach = (range.end + BUFFER).div_ceil(CHUNK) * CHUNK;
        let end = match self.total {
            0 => reach.max(range.end).max(CHUNK),
            total => reach.min(total.max(range.end)),
        };
        page(start, end.saturating_sub(start))
    }

    /// The read the viewport needs, or `None` when the one in flight already covers it.
    pub fn needs(&mut self, range: &Range<usize>) -> Option<Page> {
        let wanted = self.page_for(range);
        if self.requested == Some(wanted) {
            return None;
        }
        self.requested = Some(wanted);
        Some(wanted)
    }

    /// The first read, before anything has been laid out: one chunk from the top.
    pub fn first(&mut self) -> Page {
        let first = page(0, CHUNK);
        self.requested = Some(first);
        first
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_read_is_one_chunk_not_the_whole_library() {
        let mut pager = Pager::new();
        let first = pager.first();
        assert_eq!(first.offset, 0);
        assert_eq!(first.limit, CHUNK as u32);
        assert!(first.limit < u32::MAX);
    }

    #[test]
    fn a_window_snaps_out_to_whole_chunks() {
        let mut pager = Pager::new();
        pager.adopt(0, CHUNK, 10_000);
        let asked = pager.page_for(&(310..330));
        assert_eq!(asked.offset as usize % CHUNK, 0);
        assert!(asked.offset as usize <= 310 - BUFFER);
        assert!(asked.offset as usize + asked.limit as usize >= 330 + BUFFER);
    }

    #[test]
    fn scrolling_inside_a_window_asks_for_nothing() {
        let mut pager = Pager::new();
        pager.adopt(0, CHUNK, 10_000);
        assert!(pager.needs(&(0..20)).is_some());
        assert!(pager.needs(&(1..21)).is_none());
        assert!(pager.needs(&(2..22)).is_none());
    }

    #[test]
    fn scrolling_out_of_a_window_asks_for_the_next_one() {
        let mut pager = Pager::new();
        pager.adopt(0, CHUNK, 10_000);
        pager.needs(&(0..20));
        let next = pager.needs(&(400..420)).expect("a further window");
        assert!(next.offset > 0);
    }

    #[test]
    fn a_window_never_runs_past_the_end_of_the_listing() {
        let mut pager = Pager::new();
        pager.adopt(0, CHUNK, 120);
        let asked = pager.page_for(&(100..120));
        assert!(asked.offset as usize + asked.limit as usize <= 120);
    }

    #[test]
    fn a_row_outside_the_held_slice_is_known_to_be_missing() {
        let mut pager = Pager::new();
        pager.adopt(200, CHUNK, 10_000);
        assert!(!pager.holds(0));
        assert!(pager.holds(200));
        assert!(!pager.holds(200 + CHUNK));
    }

    #[test]
    fn a_reset_forgets_the_read_in_flight_so_the_same_window_is_asked_again() {
        let mut pager = Pager::new();
        pager.adopt(0, CHUNK, 10_000);
        pager.needs(&(0..20));
        pager.reset();
        assert_eq!(pager.total(), 0);
        assert!(pager.needs(&(0..20)).is_some());
    }
}
