//! A mark that travels to the tab it marks, instead of appearing under a different one.
//!
//! GPUI will not say where a laid-out element ended up, so each tab carries a probe that
//! reports its own bounds and the strip carries one that reports where the tabs start.

use gpui_kit::{App, Bounds, ElementId, Entity, IntoElement, Pixels, SharedString, Styled, Window};

use crate::motion;

/// Which way the tabs run, and therefore which edge the mark slides along.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Along {
    Row,
    Column,
}

impl Along {
    fn start(self, bounds: &Bounds<Pixels>) -> Pixels {
        match self {
            Along::Row => bounds.origin.x,
            Along::Column => bounds.origin.y,
        }
    }

    fn extent(self, bounds: &Bounds<Pixels>) -> Pixels {
        match self {
            Along::Row => bounds.size.width,
            Along::Column => bounds.size.height,
        }
    }
}

/// Where each tab sits along the strip, and where the strip's own tabs begin.
#[derive(Default)]
pub struct Marks {
    held: Vec<(SharedString, Pixels, Pixels)>,
    origin: Pixels,
}

impl Marks {
    /// Where `key`'s tab sits, relative to the strip rather than the window.
    pub fn of(&self, key: &str) -> Option<(Pixels, Pixels)> {
        self.held
            .iter()
            .find(|(held, _, _)| held.as_ref() == key)
            .map(|(_, start, extent)| (*start - self.origin, *extent))
    }

    fn put(&mut self, key: &SharedString, start: Pixels, extent: Pixels) -> bool {
        match self.held.iter_mut().find(|(held, _, _)| held == key) {
            Some(entry) if entry.1 == start && entry.2 == extent => false,
            Some(entry) => {
                entry.1 = start;
                entry.2 = extent;
                true
            }
            None => {
                self.held.push((key.clone(), start, extent));
                true
            }
        }
    }

    fn set_origin(&mut self, origin: Pixels) -> bool {
        let moved = self.origin != origin;
        self.origin = origin;
        moved
    }
}

/// The measurements for one strip, kept across frames.
pub fn marks(id: impl Into<ElementId>, window: &mut Window, cx: &mut App) -> Entity<Marks> {
    window.use_keyed_state(id.into(), cx, |_, _| Marks::default())
}

/// Put this first inside the strip, in the flow, so a scrolled strip moves it with the tabs.
pub fn origin(along: Along, marks: Entity<Marks>) -> impl IntoElement {
    gpui_kit::canvas(
        move |bounds, _, cx| {
            marks.update(cx, |marks, cx| {
                if marks.set_origin(along.start(&bounds)) {
                    cx.notify();
                }
            });
        },
        |_, _, _, _| {},
    )
    .w_0()
    .h_0()
    .flex_none()
}

/// Put this inside a tab that carries **no padding of its own**.
///
/// An absolute child is placed against the content box, so a padded tab reports itself inset
/// and the mark then tracks a box the tab is not drawn in. Pad a wrapper inside instead.
pub fn probe(key: impl Into<SharedString>, along: Along, marks: Entity<Marks>) -> impl IntoElement {
    let key = key.into();
    gpui_kit::canvas(
        move |bounds, _, cx| {
            marks.update(cx, |marks, cx| {
                if marks.put(&key, along.start(&bounds), along.extent(&bounds)) {
                    cx.notify();
                }
            });
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_full()
}

/// Where the mark should be drawn this frame, eased toward the tab it marks.
pub fn slide(
    id: &'static str,
    marks: &Entity<Marks>,
    key: &str,
    window: &mut Window,
    cx: &mut App,
) -> (Pixels, Pixels) {
    let measured = marks.read(cx).of(key);
    let start = motion::length(
        ElementId::from((ElementId::from(id), "start")),
        measured.map(|held| held.0).unwrap_or_default(),
        motion::MEDIUM,
        window,
        cx,
    );
    let extent = motion::length(
        ElementId::from((ElementId::from(id), "extent")),
        measured.map(|held| held.1).unwrap_or_default(),
        motion::MEDIUM,
        window,
        cx,
    );
    (start, extent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{point, px, size};

    fn at(x: f32, width: f32) -> Bounds<Pixels> {
        Bounds {
            origin: point(px(x), px(0.)),
            size: size(px(width), px(44.)),
        }
    }

    #[test]
    fn a_tab_is_placed_against_the_strip_not_the_window() {
        let mut marks = Marks::default();
        marks.set_origin(px(24.));
        marks.put(&"interface".into(), px(235.), px(92.));
        assert_eq!(marks.of("interface"), Some((px(211.), px(92.))));
    }

    #[test]
    fn a_scrolled_strip_carries_its_origin_along_with_its_tabs() {
        let mut marks = Marks::default();
        marks.set_origin(px(-40.));
        marks.put(&"interface".into(), px(171.), px(92.));
        assert_eq!(marks.of("interface"), Some((px(211.), px(92.))));
    }

    #[test]
    fn a_column_measures_the_other_axis() {
        let bounds = Bounds {
            origin: point(px(8.), px(96.)),
            size: size(px(200.), px(40.)),
        };
        assert_eq!(Along::Column.start(&bounds), px(96.));
        assert_eq!(Along::Column.extent(&bounds), px(40.));
        assert_eq!(Along::Row.start(&at(12., 50.)), px(12.));
        assert_eq!(Along::Row.extent(&at(12., 50.)), px(50.));
    }

    #[test]
    fn a_tab_nobody_has_measured_yet_places_nothing() {
        assert_eq!(Marks::default().of("tools"), None);
    }

    #[test]
    fn measuring_the_same_place_twice_asks_for_no_redraw() {
        let mut marks = Marks::default();
        assert!(marks.put(&"home".into(), px(10.), px(80.)));
        assert!(!marks.put(&"home".into(), px(10.), px(80.)));
        assert!(marks.put(&"home".into(), px(12.), px(80.)));
    }
}
