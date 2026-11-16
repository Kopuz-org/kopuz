//! Frame demand shared between GPUI's window invalidator and a host-driven
//! frame source (a `CADisplayLink` on iOS).

use std::{cell::Cell, ffi::c_void};

/// A host callback that resumes the frame source, with its context pointer.
pub(crate) type HostFrameWaker = (unsafe extern "C" fn(*mut c_void), *mut c_void);

/// Frame demand, shared between GPUI's window invalidator (through
/// `PlatformWindow::frame_waker` and `PlatformWindow::schedule_frame`) and
/// the host's display link (through `gpui_ios_request_frame`).
///
/// GPUI asks for a frame when a view is notified, when an animation wants the
/// next frame, or when a frame it just drew left the window dirty. Without
/// this, a host has to tick GPUI on every vsync and GPUI decides each time
/// whether there is anything to draw; with it, the host can pause its frame
/// source after a tick that produced no demand and resume it from the waker,
/// so an idle screen costs no CPU at all.
///
/// Main thread only, like everything in GPUI.
#[derive(Default)]
pub(crate) struct FrameDemand {
    /// A frame has been asked for since the host last ticked.
    pending: Cell<bool>,
    host_waker: Cell<Option<HostFrameWaker>>,
}

impl FrameDemand {
    /// Records demand and, on the first demand since the last tick, resumes
    /// the host's frame source.
    pub(crate) fn wake(&self) {
        if self.pending.replace(true) {
            return;
        }
        if let Some((waker, context)) = self.host_waker.get() {
            // Safety: the host registered this pair and guarantees it stays
            // valid until it clears the waker.
            unsafe { waker(context) };
        }
    }

    /// Clears the demand at the start of a tick; a frame drawn in this tick
    /// re-raises it if it wants another.
    pub(crate) fn clear(&self) {
        self.pending.set(false);
    }

    pub(crate) fn is_pending(&self) -> bool {
        self.pending.get()
    }

    /// Registers (or, with `None`, clears) the host's waker. Demand that
    /// arrived before the host registered is delivered right away.
    pub(crate) fn set_host_waker(&self, waker: Option<HostFrameWaker>) {
        self.host_waker.set(waker);
        if let (true, Some((waker, context))) = (self.pending.get(), waker) {
            // Safety: as in `wake`, the host keeps the pair valid until it
            // clears the waker.
            unsafe { waker(context) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::ffi::c_void;

    unsafe extern "C" fn count_wakes(context: *mut c_void) {
        // Safety: tests register a `Cell<u32>` and keep it alive for the demand.
        let counter = unsafe { &*(context as *const Cell<u32>) };
        counter.set(counter.get() + 1);
    }

    fn demand_with_counter(counter: &Cell<u32>) -> FrameDemand {
        let demand = FrameDemand::default();
        demand.set_host_waker(Some((count_wakes, counter as *const _ as *mut c_void)));
        demand
    }

    #[test]
    fn wake_calls_host_once_per_tick() {
        let wakes = Cell::new(0);
        let demand = demand_with_counter(&wakes);

        demand.wake();
        demand.wake();

        assert!(demand.is_pending());
        assert_eq!(wakes.get(), 1);
    }

    #[test]
    fn clear_resets_demand_so_next_wake_calls_host_again() {
        let wakes = Cell::new(0);
        let demand = demand_with_counter(&wakes);

        demand.wake();
        demand.clear();
        assert!(!demand.is_pending());

        demand.wake();
        assert!(demand.is_pending());
        assert_eq!(wakes.get(), 2);
    }

    #[test]
    fn registering_host_with_pending_demand_wakes_immediately() {
        let wakes = Cell::new(0);
        let demand = FrameDemand::default();

        demand.wake();
        assert_eq!(wakes.get(), 0);

        demand.set_host_waker(Some((count_wakes, &wakes as *const _ as *mut c_void)));
        assert_eq!(wakes.get(), 1);
    }

    #[test]
    fn registering_host_without_demand_does_not_wake() {
        let wakes = Cell::new(0);
        let _demand = demand_with_counter(&wakes);
        assert_eq!(wakes.get(), 0);
    }

    #[test]
    fn clearing_host_waker_keeps_recording_demand() {
        let wakes = Cell::new(0);
        let demand = demand_with_counter(&wakes);

        demand.set_host_waker(None);
        demand.wake();

        assert!(demand.is_pending());
        assert_eq!(wakes.get(), 0);
    }
}
