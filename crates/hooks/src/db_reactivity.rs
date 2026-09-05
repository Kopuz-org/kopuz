//! Reactive plumbing over daemon library invalidation events.
//!
//! Per-table generation counters let API query hooks re-run when their resource
//! changes without holding the data in a giant signal.
//!
//! Coalescing matters for bulk jobs. The first invalidation arms a one-shot
//! flush, and later invalidations join it without an idle periodic timer.

use dioxus::prelude::*;

/// The daemon resources the UI observes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Table {
    Tracks = 0,
    Albums = 1,
    Playlists = 2,
    Favorites = 3,
    Folders = 4,
    Servers = 5,
    Recents = 6,
}

const N: usize = 7;

/// One monotonically-increasing counter per [`Table`], plus a dirty bitset the
/// flusher drains. `Copy` (just `Signal`s inside), so it's cheap to pass around
/// and capture in closures.
#[derive(Clone, Copy)]
pub struct Generations {
    counters: [Signal<u64>; N],
    dirty: Signal<[bool; N]>,
    flush_scheduled: Signal<bool>,
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
        if *self.flush_scheduled.peek() {
            return;
        }
        self.flush_scheduled.set(true);
        spawn(async move {
            utils::sleep(std::time::Duration::from_millis(150)).await;
            self.flush_scheduled.set(false);
            self.flush();
        });
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
            use_signal(|| 0u64),
        ],
        dirty: use_signal(|| [false; N]),
        flush_scheduled: use_signal(|| false),
    };
    use_context_provider(|| gens);

    gens
}

/// Read the provided [`Generations`] from context.
pub fn use_generations() -> Generations {
    use_context::<Generations>()
}
