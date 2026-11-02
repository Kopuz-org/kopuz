//! Where the window was left, so it opens there again.

use gpui_kit::{App, Bounds, Point, Size, Window, WindowBounds, px};
use store::LayoutModel;
use store::layout::WindowState;

/// The bounds the window should open at, or nothing when it has never been placed.
pub fn saved_bounds(cx: &App) -> Option<WindowBounds> {
    let held = store::layout(cx).window;
    let size = Size {
        width: px(held.width?),
        height: px(held.height?),
    };
    let origin = Point {
        x: px(held.x.unwrap_or_default()),
        y: px(held.y.unwrap_or_default()),
    };
    let bounds = Bounds { origin, size };
    Some(match held.maximized {
        true => WindowBounds::Maximized(bounds),
        false => WindowBounds::Windowed(bounds),
    })
}

/// Write down where the window is now; the layout file only catches up once resizing settles.
pub fn remember(window: &Window, cx: &mut App) {
    let (bounds, maximized) = match window.window_bounds() {
        WindowBounds::Windowed(bounds) => (bounds, false),
        WindowBounds::Maximized(bounds) => (bounds, true),
        // A fullscreen window is not where it wants to open next time.
        WindowBounds::Fullscreen(_) => return,
    };
    let state = WindowState {
        width: Some(bounds.size.width.as_f32()),
        height: Some(bounds.size.height.as_f32()),
        x: Some(bounds.origin.x.as_f32()),
        y: Some(bounds.origin.y.as_f32()),
        maximized,
    };
    if store::layout(cx).window == state {
        return;
    }
    LayoutModel::edit(cx, |layout| layout.window = state);
}
