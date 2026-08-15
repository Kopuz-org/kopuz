//! One-finger swipe tracking for the touch layouts.
//!
//! Dioxus only hands us raw `touchstart`/`touchmove`/`touchend`, so this keeps
//! the origin and the live delta in signals. The delta is what lets a surface
//! follow the finger while it drags — a sheet that only jumps at the end of the
//! gesture reads as a tap, not a drag.

use dioxus::prelude::*;

/// The axis-dominant direction of a completed swipe.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SwipeDirection {
    Up,
    Down,
    Left,
    Right,
}

/// Travel (px) a gesture needs before it counts as a swipe rather than a tap.
pub const SWIPE_THRESHOLD: f64 = 64.0;

/// How much the dominant axis has to beat the other one by. Without this a
/// lazy diagonal drag fires whichever direction happened to win by a pixel.
const AXIS_DOMINANCE: f64 = 1.5;

/// Tracks one finger across a touch sequence. Cheap to copy — it is only a pair
/// of signals — so it can be moved into every handler closure a node needs.
#[derive(Clone, Copy)]
pub struct Swipe {
    origin: Signal<Option<(f64, f64)>>,
    delta: Signal<(f64, f64)>,
}

pub fn use_swipe() -> Swipe {
    Swipe {
        origin: use_signal(|| None),
        delta: use_signal(|| (0.0, 0.0)),
    }
}

fn first_touch(evt: &TouchEvent) -> Option<(f64, f64)> {
    // `touches` is empty on touchend (the finger is gone by then), so fall back
    // to the points that changed in this event.
    let touches = evt.touches();
    let changed = evt.touches_changed();
    let point = touches.first().or_else(|| changed.first())?;
    let at = point.client_coordinates();
    Some((at.x, at.y))
}

impl Swipe {
    /// Begin tracking. `on_touch_start`.
    pub fn start(&mut self, evt: &TouchEvent) {
        self.origin.set(first_touch(evt));
        self.delta.set((0.0, 0.0));
    }

    /// Update the live delta. `on_touch_move`.
    pub fn update(&mut self, evt: &TouchEvent) {
        let Some((ox, oy)) = *self.origin.peek() else {
            return;
        };
        let Some((x, y)) = first_touch(evt) else {
            return;
        };
        self.delta.set((x - ox, y - oy));
    }

    /// Finish tracking and report the direction, if the gesture travelled far
    /// enough on one dominant axis. Resets either way. `on_touch_end`.
    pub fn finish(&mut self, evt: &TouchEvent) -> Option<SwipeDirection> {
        self.update(evt);
        let (dx, dy) = *self.delta.peek();
        self.reset();

        if dx.abs() > dy.abs() * AXIS_DOMINANCE && dx.abs() >= SWIPE_THRESHOLD {
            return Some(if dx > 0.0 {
                SwipeDirection::Right
            } else {
                SwipeDirection::Left
            });
        }
        if dy.abs() > dx.abs() * AXIS_DOMINANCE && dy.abs() >= SWIPE_THRESHOLD {
            return Some(if dy > 0.0 {
                SwipeDirection::Down
            } else {
                SwipeDirection::Up
            });
        }
        None
    }

    /// Drop the gesture without reporting anything — for `touchcancel`, which
    /// the system fires when it takes the finger over (back gesture, notification
    /// shade). Without this the surface stays stuck at its dragged offset.
    pub fn reset(&mut self) {
        self.origin.set(None);
        self.delta.set((0.0, 0.0));
    }

    /// Where the finger started, in client coordinates.
    pub fn origin(&self) -> Option<(f64, f64)> {
        *self.origin.read()
    }

    /// Live travel from the origin. `(0.0, 0.0)` when nothing is in flight.
    pub fn delta(&self) -> (f64, f64) {
        *self.delta.read()
    }

    /// Downward travel only, clamped at zero — the offset for a pull-to-dismiss
    /// sheet, which should not follow a finger dragging back up past the top.
    pub fn pull_down(&self) -> f64 {
        self.delta().1.max(0.0)
    }
}
