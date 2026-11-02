//! One tab stop per group, arrows within it: the keyboard shape every strip, list and grid wears.

use gpui_kit::base::FocusableExt as _;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::Button;
use gpui_kit::{
    AnyElement, App, Div, Entity, FocusHandle, Global, InteractiveElement as _, IntoElement,
    KeyDownEvent, Rems, SharedString, Stateful, Styled as _, Window, div,
};

/// How the arrows walk a group: one row, one column, or a grid that many columns wide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Row,
    Column,
    Grid(usize),
}

/// What a keystroke did, so the caller can scroll, open, or decline it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Moved(usize),
    Activate(usize),
    Ignored,
}

/// A group of things sharing one tab stop; the cursor inside it is what the arrows move.
pub struct Roving {
    focus: FocusHandle,
    axis: Axis,
    cursor: usize,
    len: usize,
}

impl Roving {
    pub fn new(axis: Axis, cx: &mut App) -> Self {
        Self {
            focus: stop_handle(cx),
            axis,
            cursor: 0,
            len: 0,
        }
    }

    /// How many things the group holds now; the cursor is clamped to them.
    pub fn set_len(&mut self, len: usize) {
        self.len = len;
        self.cursor = self.cursor.min(len.saturating_sub(1));
    }

    /// Widen or narrow a grid as its container resizes.
    pub fn set_axis(&mut self, axis: Axis) {
        self.axis = axis;
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn set_cursor(&mut self, index: usize) {
        self.cursor = index.min(self.len.saturating_sub(1));
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus.is_focused(window)
    }

    /// Whether `index` should wear the ring: the group holds focus and the cursor is there.
    pub fn is_active(&self, index: usize, window: &Window) -> bool {
        self.cursor == index && self.focus.is_focused(window)
    }

    /// The group frame: one tab stop for the whole strip, plus the context its arrows are bound in.
    pub fn group(&self, element: Stateful<Div>, context: &'static str) -> Stateful<Div> {
        element
            .track_focus(&self.focus)
            .key_context(context)
            .tab_index(0)
    }

    /// Move the cursor for `event` and say what happened; `Ignored` means let the key through.
    pub fn walk(&mut self, event: &KeyDownEvent) -> Step {
        let step = step(
            self.axis,
            self.cursor,
            self.len,
            event.keystroke.key.as_str(),
        );
        if let Step::Moved(cursor) = step {
            self.cursor = cursor;
        }
        step
    }
}

/// Where one keystroke takes a cursor; the whole of the arrow walk, with nothing to render.
fn step(axis: Axis, cursor: usize, len: usize, key: &str) -> Step {
    if len == 0 {
        return Step::Ignored;
    }
    let last = len - 1;
    let stride = match axis {
        Axis::Grid(columns) => columns.max(1),
        _ => 1,
    };
    let moved = match (axis, key) {
        (_, "enter") | (_, "space") => return Step::Activate(cursor),
        (_, "home") => 0,
        (_, "end") => last,
        (Axis::Row, "left") | (Axis::Column, "up") | (Axis::Grid(_), "left") => {
            cursor.saturating_sub(1)
        }
        (Axis::Row, "right") | (Axis::Column, "down") | (Axis::Grid(_), "right") => {
            (cursor + 1).min(last)
        }
        (Axis::Grid(_), "up") => cursor.saturating_sub(stride),
        (Axis::Grid(_), "down") => (cursor + stride).min(last),
        _ => return Step::Ignored,
    };
    Step::Moved(moved)
}

/// Which device is driving focus. A ring answers "where will the keys land", which a pointer
/// never has to ask: clicking a button and being left with a ring on it is the web's old
/// `:focus` rather than its `:focus-visible`.
#[derive(Default)]
struct Modality {
    keyboard: bool,
}

impl Global for Modality {}

/// Whether focus is being moved by the keyboard, and so whether a ring is worth drawing.
pub fn keyboard(cx: &App) -> bool {
    cx.try_global::<Modality>()
        .is_some_and(|modality| modality.keyboard)
}

fn drive(keyboard: bool, cx: &mut App) {
    if self::keyboard(cx) == keyboard {
        return;
    }
    cx.default_global::<Modality>().keyboard = keyboard;
    // Rings are drawn wherever focus is, so the switch is a whole-window repaint; it is rare.
    cx.refresh_windows();
}

/// Watch a root for what is driving focus. Every ring in the window follows it.
pub fn watch(root: Div) -> Div {
    root.capture_any_mouse_down(|_, _, cx| drive(false, cx))
        .capture_key_down(|_, _, cx| drive(true, cx))
}

/// A button that rings for the keyboard and stays quiet under a click.
pub fn quiet(button: Button, cx: &App) -> Button {
    button.focus_ring(keyboard(cx))
}

/// The focus ring, drawn over a thing rather than around it, so focusing never moves anything.
pub fn ring(radius: Rems, cx: &App) -> AnyElement {
    div()
        .absolute()
        .inset_0()
        .rounded(radius)
        .border_1()
        .border_color(cx.theme().ring)
        .into_any_element()
}

/// A handle that is a tab stop: `track_focus` keeps the handle's own flags, never the div's.
pub fn stop_handle(cx: &mut App) -> FocusHandle {
    cx.focus_handle().tab_stop(true)
}

/// A focus handle for an element with no view to hold one; keyed by id, kept across frames.
pub fn handle(key: impl Into<SharedString>, window: &mut Window, cx: &mut App) -> FocusHandle {
    let held: Entity<FocusHandle> = window.use_keyed_state(key.into(), cx, |_, cx| stop_handle(cx));
    held.read(cx).clone()
}

/// One tab stop that is not part of a group: a card, a tile, or the bar's search field.
pub fn stop(
    element: Stateful<Div>,
    focus: &FocusHandle,
    open: impl Fn(&mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    element
        .track_focus(focus)
        .tab_index(0)
        .on_key_down(move |event: &KeyDownEvent, window, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                open(window, cx);
            }
        })
}

/// A hover-only affordance rests here rather than vanishing: `Div::paint` drops a popup it hides.
pub const RESTING: f32 = 0.4;

/// What a dots button or a hover glyph is drawn at, revealed by hover *or* focus.
pub fn reveal(shown: bool) -> f32 {
    match shown {
        true => 1.,
        false => RESTING,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Walk a cursor the way `Roving` does, without a window to hold a focus handle.
    fn walk(axis: Axis, cursor: &mut usize, len: usize, key: &str) -> Step {
        let step = step(axis, *cursor, len, key);
        if let Step::Moved(moved) = step {
            *cursor = moved;
        }
        step
    }

    #[test]
    fn a_row_walks_with_left_and_right_and_stops_at_the_ends() {
        let mut at = 0;
        assert_eq!(walk(Axis::Row, &mut at, 3, "left"), Step::Moved(0));
        assert_eq!(walk(Axis::Row, &mut at, 3, "right"), Step::Moved(1));
        assert_eq!(walk(Axis::Row, &mut at, 3, "end"), Step::Moved(2));
        assert_eq!(walk(Axis::Row, &mut at, 3, "right"), Step::Moved(2));
    }

    #[test]
    fn a_column_declines_the_horizontal_arrows() {
        let mut at = 0;
        assert_eq!(walk(Axis::Column, &mut at, 3, "down"), Step::Moved(1));
        assert_eq!(walk(Axis::Column, &mut at, 3, "left"), Step::Ignored);
        assert_eq!(walk(Axis::Column, &mut at, 3, "enter"), Step::Activate(1));
    }

    #[test]
    fn a_grid_moves_a_whole_band_at_a_time() {
        let grid = Axis::Grid(4);
        let mut at = 0;
        assert_eq!(walk(grid, &mut at, 10, "down"), Step::Moved(4));
        assert_eq!(walk(grid, &mut at, 10, "down"), Step::Moved(8));
        assert_eq!(walk(grid, &mut at, 10, "down"), Step::Moved(9));
        assert_eq!(walk(grid, &mut at, 10, "up"), Step::Moved(5));
    }

    #[test]
    fn an_empty_group_swallows_nothing() {
        let mut at = 0;
        assert_eq!(walk(Axis::Row, &mut at, 0, "right"), Step::Ignored);
        assert_eq!(walk(Axis::Row, &mut at, 0, "enter"), Step::Ignored);
    }
}
