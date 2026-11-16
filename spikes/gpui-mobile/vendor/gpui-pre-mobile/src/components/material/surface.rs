//! Material surface component — elevated card base with shadow simulation.
//!
//! The [`surface`] function returns a `Div` container styled as a Material
//! Design 3 elevated card. The `elevation` parameter (0–3) controls the
//! background tint and border opacity, simulating the layered surface
//! system described in the MD3 specification.
//!
//! In dark mode, higher elevation maps to progressively lighter surface
//! tones. In light mode, the surface is always white and the border
//! becomes slightly more visible at higher elevations.

use gpui::{div, hsla, prelude::*, px, rgb};

/// Returns a Material Design surface container with the given elevation.
///
/// The container is a rounded, bordered `Div` with `overflow_hidden` — you
/// can append children with `.child(...)` before the element is finalised.
///
/// # Elevation levels
///
/// | Level | Dark-mode background | Description |
/// |-------|---------------------|-------------|
/// | 0     | `#121212`           | Base surface |
/// | 1     | `#1e1e1e`           | Low elevation (cards) |
/// | 2     | `#232323`           | Medium elevation (dialogs) |
/// | 3+    | `#282828`           | High elevation (FABs, nav drawers) |
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::material;
///
/// let card = material::surface(true, 2) // dark mode, elevation 2
///     .child(content)
///     .child(actions);
/// ```
pub fn surface(dark: bool, elevation: u8) -> gpui::Div {
    let bg = if dark {
        match elevation {
            0 => rgb(0x121212),
            1 => rgb(0x1e1e1e),
            2 => rgb(0x232323),
            _ => rgb(0x282828),
        }
    } else {
        rgb(0xffffff)
    };
    let border = if dark {
        hsla(0.0, 0.0, 1.0, 0.04 * elevation as f32)
    } else {
        hsla(0.0, 0.0, 0.0, 0.04 * elevation as f32)
    };

    div()
        .flex()
        .flex_col()
        .rounded(px(12.0))
        .bg(bg)
        .border_1()
        .border_color(border)
        .overflow_hidden()
}
