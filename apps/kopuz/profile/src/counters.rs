//! What the app is holding, counted where it is created rather than guessed from RSS.

use std::sync::atomic::Ordering;

use crate::{
    ARTWORK_BYTES, ARTWORK_DECODED, ARTWORK_LIVE, ARTWORK_LOADED, ARTWORK_RELEASED,
    ARTWORK_TRACKED, ELEMENTS, NOTIFIES, REBUILDS,
};

/// One reading of every counter.
#[derive(Debug, Clone, Copy, Default)]
pub struct Snapshot {
    pub artwork_live: usize,
    pub artwork_tracked: usize,
    pub artwork_loaded: u64,
    pub artwork_released: u64,
    pub artwork_bytes: u64,
    pub artwork_decoded: u64,
    pub rebuilds: u64,
    pub notifies: u64,
    pub elements: u64,
}

pub struct Counters;

impl Counters {
    pub fn snapshot() -> Snapshot {
        Snapshot {
            artwork_live: ARTWORK_LIVE.load(Ordering::Relaxed),
            artwork_tracked: ARTWORK_TRACKED.load(Ordering::Relaxed),
            artwork_loaded: ARTWORK_LOADED.load(Ordering::Relaxed),
            artwork_released: ARTWORK_RELEASED.load(Ordering::Relaxed),
            artwork_bytes: ARTWORK_BYTES.load(Ordering::Relaxed),
            artwork_decoded: ARTWORK_DECODED.load(Ordering::Relaxed),
            rebuilds: REBUILDS.load(Ordering::Relaxed),
            notifies: NOTIFIES.load(Ordering::Relaxed),
            elements: ELEMENTS.load(Ordering::Relaxed),
        }
    }
}

/// Artwork accounting, called from the asset loader.
pub mod artwork {
    use super::*;

    /// One cover arrived: `encoded` is what the daemon sent, `width`/`height` what it decodes to.
    pub fn loaded(encoded: usize, width: u32, height: u32) {
        ARTWORK_LIVE.fetch_add(1, Ordering::Relaxed);
        ARTWORK_LOADED.fetch_add(1, Ordering::Relaxed);
        ARTWORK_BYTES.fetch_add(encoded as u64, Ordering::Relaxed);
        // Every decoded cover is held as RGBA, whatever it was encoded as.
        ARTWORK_DECODED.fetch_add(u64::from(width) * u64::from(height) * 4, Ordering::Relaxed);
    }

    /// One cover was swept out of every cache holding it.
    pub fn released() {
        ARTWORK_LIVE.fetch_sub(1, Ordering::Relaxed);
        ARTWORK_RELEASED.fetch_add(1, Ordering::Relaxed);
    }

    /// How many covers the sweep can actually reach. Below `live`, something loaded is unreachable.
    pub fn tracked(count: usize) {
        ARTWORK_TRACKED.store(count, Ordering::Relaxed);
    }
}

/// How many times the view tree has been rebuilt, which is the notify-storm signal.
pub mod rebuilds {
    use super::*;

    pub fn count() -> u64 {
        REBUILDS.load(Ordering::Relaxed)
    }
}

/// Every `cx.notify()` on a shared model, which is the storm that rebuilds the window.
pub mod notifies {
    use super::*;

    /// Call it beside the notify, not instead of it.
    pub fn record() {
        NOTIFIES.fetch_add(1, Ordering::Relaxed);
    }

    pub fn count() -> u64 {
        NOTIFIES.load(Ordering::Relaxed)
    }
}

/// Elements a frame builds, counted where building one costs something: a list row, a grid tile.
pub mod elements {
    use super::*;

    pub fn built() {
        ELEMENTS.fetch_add(1, Ordering::Relaxed);
    }

    pub fn count() -> u64 {
        ELEMENTS.load(Ordering::Relaxed)
    }
}

/// How many decoded covers are alive, which is the number that explains the resident set.
pub mod images {
    use super::*;

    pub fn live() -> usize {
        ARTWORK_LIVE.load(Ordering::Relaxed)
    }
}
