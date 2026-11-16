//! iOS event handling - converting UIKit events to GPUI's event types.
//!
//! iOS uses touch-based input rather than mouse input, so we need to map
//! touch gestures to appropriate GPUI events:
//! - Single tap → MouseDown + MouseUp (left button)
//! - Pan gesture → ScrollWheel events
//! - Touch move → MouseMove events

use gpui::{px, Pixels, Point, TouchPhase};
use objc2::msg_send;
use objc2::runtime::AnyObject;

use super::cg_types::ObjcCGPoint;

/// Touch phase from UIKit
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i64)]
pub enum UITouchPhase {
    Began = 0,
    Moved = 1,
    Stationary = 2,
    Ended = 3,
    Cancelled = 4,
}

impl From<i64> for UITouchPhase {
    fn from(value: i64) -> Self {
        match value {
            0 => UITouchPhase::Began,
            1 => UITouchPhase::Moved,
            2 => UITouchPhase::Stationary,
            3 => UITouchPhase::Ended,
            4 => UITouchPhase::Cancelled,
            _ => UITouchPhase::Cancelled,
        }
    }
}

impl From<UITouchPhase> for TouchPhase {
    fn from(phase: UITouchPhase) -> Self {
        match phase {
            UITouchPhase::Began => TouchPhase::Started,
            UITouchPhase::Moved => TouchPhase::Moved,
            UITouchPhase::Stationary => TouchPhase::Moved,
            UITouchPhase::Ended => TouchPhase::Ended,
            UITouchPhase::Cancelled => TouchPhase::Ended,
        }
    }
}

/// Convert a UITouch to a mouse position
pub fn touch_location_in_view(touch: *mut AnyObject, view: *mut AnyObject) -> Point<Pixels> {
    unsafe {
        let location: ObjcCGPoint = msg_send![touch, locationInView: view];
        Point::new(px(location.x as f32), px(location.y as f32))
    }
}

/// Get the touch phase from a UITouch
pub fn touch_phase(touch: *mut AnyObject) -> UITouchPhase {
    unsafe {
        let phase: i64 = msg_send![touch, phase];
        UITouchPhase::from(phase)
    }
}
