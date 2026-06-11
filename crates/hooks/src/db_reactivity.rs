//! Reactive plumbing over the DB (issue #347, step 5).
//!
//! Per-table generation counters let DB query hooks ([`use_resource`]-based, in
//! `use_db_queries`) re-run when their table changes, without holding the data
//! in a giant signal. A writer calls [`Generations::bump`] (or, for streaming
//! inserts, [`Generations::bump_coalesced`]) after committing; any query keyed on
//! that table's counter re-runs.
//!
//! Coalescing matters for bulk writes: a 20k-row scan that bumped on every batch
//! would re-render the UI thousands of times. `bump_coalesced` instead sets a
//! dirty flag that a single ~150ms ticker flushes, capping refreshes at ~6/sec.

use dioxus::prelude::*;

/// The DB tables the UI observes. `as usize` indexes the counter/dirty arrays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Table {
    Tracks = 0,
    Albums = 1,
    Playlists = 2,
    Favorites = 3,
    Folders = 4,
    Servers = 5,
}

const N: usize = 6;

/// One monotonically-increasing counter per [`Table`], plus a dirty bitset the
/// flusher drains. `Copy` (just `Signal`s inside), so it's cheap to pass around
/// and capture in closures.
#[derive(Clone, Copy)]
pub struct Generations {
    counters: [Signal<u64>; N],
    dirty: Signal<[bool; N]>,
}

impl Generations {
    /// Bump immediately — the keyed queries re-run on the next render. Use for
    /// one-shot mutations (a favorite toggle, a single upsert).
    pub fn bump(mut self, table: Table) {
        *self.counters[table as usize].write() += 1;
    }

    /// Mark the table dirty; the ticker coalesces it into a single bump within
    /// ~150ms. Use on the hot path of a streaming insert (scan/sync batches).
    pub fn bump_coalesced(mut self, table: Table) {
        self.dirty.write()[table as usize] = true;
    }

    /// Current generation of a table. Read this inside a query hook so the hook
    /// is subscribed and re-runs on bump.
    pub fn generation(self, table: Table) -> u64 {
        (self.counters[table as usize])()
    }

    /// Drain dirty flags into real bumps. Called by the ticker; one write per
    /// dirty table, nothing when idle.
    fn flush(mut self) {
        let dirty = *self.dirty.peek();
        if !dirty.iter().any(|&d| d) {
            return;
        }
        for (i, &is_dirty) in dirty.iter().enumerate() {
            if is_dirty {
                *self.counters[i].write() += 1;
            }
        }
        self.dirty.set([false; N]);
    }
}

/// Create the [`Generations`], provide it via context, and install the single
/// coalescing ticker. Call once, high in the tree (e.g. `App`).
pub fn use_generations_provider() -> Generations {
    let gens = Generations {
        counters: [
            use_signal(|| 0u64),
            use_signal(|| 0u64),
            use_signal(|| 0u64),
            use_signal(|| 0u64),
            use_signal(|| 0u64),
            use_signal(|| 0u64),
        ],
        dirty: use_signal(|| [false; N]),
    };
    use_context_provider(|| gens);

    use_future(move || async move {
        loop {
            utils::sleep(std::time::Duration::from_millis(150)).await;
            gens.flush();
        }
    });

    gens
}

/// Read the provided [`Generations`] from context.
pub fn use_generations() -> Generations {
    use_context::<Generations>()
}
