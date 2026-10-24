//! Allocator truth. jemalloc reports what is actually held, which RSS only approximates.

/// Reads the allocator's own figures; without jemalloc it reports zeroes rather than lying.
pub struct Heap {
    #[cfg(feature = "jemalloc")]
    allocated: tikv_jemalloc_ctl::stats::allocated_mib,
    #[cfg(feature = "jemalloc")]
    resident: tikv_jemalloc_ctl::stats::resident_mib,
    #[cfg(feature = "jemalloc")]
    epoch: tikv_jemalloc_ctl::epoch_mib,
}

impl Default for Heap {
    fn default() -> Self {
        Self::new()
    }
}

impl Heap {
    #[cfg(feature = "jemalloc")]
    pub fn new() -> Self {
        Self {
            allocated: tikv_jemalloc_ctl::stats::allocated::mib()
                .expect("jemalloc reports allocated"),
            resident: tikv_jemalloc_ctl::stats::resident::mib().expect("jemalloc reports resident"),
            epoch: tikv_jemalloc_ctl::epoch::mib().expect("jemalloc advances its stats epoch"),
        }
    }

    #[cfg(not(feature = "jemalloc"))]
    pub fn new() -> Self {
        Self {}
    }

    /// `(allocated, resident)` in bytes; jemalloc caches its stats until the epoch advances.
    #[cfg(feature = "jemalloc")]
    pub fn read(&self) -> (u64, u64) {
        let _ = self.epoch.advance();
        (
            self.allocated.read().unwrap_or(0) as u64,
            self.resident.read().unwrap_or(0) as u64,
        )
    }

    #[cfg(not(feature = "jemalloc"))]
    pub fn read(&self) -> (u64, u64) {
        (0, 0)
    }
}
