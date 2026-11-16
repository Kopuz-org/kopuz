//! Glass panel and separator primitives.
//!
//! These are the foundational building blocks for the Apple Glass design
//! language. The [`panel`] function returns a frosted-glass container that
//! other glass components are built on top of.

use gpui::{div, hsla, prelude::*, px};

// ── Panel ────────────────────────────────────────────────────────────────────

/// Frosted-glass base container — translucent background with a subtle border.
///
/// Returns a `Div` (not `impl IntoElement`) so callers can append children
/// with `.child(...)` before the element is finalised.
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::glass;
///
/// let card = glass::panel(dark)
///     .child(some_content)
///     .child(glass::separator(dark))
///     .child(more_content);
/// ```
pub fn panel(dark: bool) -> gpui::Div {
    let bg = if dark {
        hsla(0.0, 0.0, 1.0, 0.06)
    } else {
        hsla(0.0, 0.0, 1.0, 0.72)
    };
    let border = if dark {
        hsla(0.0, 0.0, 1.0, 0.1)
    } else {
        hsla(0.0, 0.0, 0.0, 0.06)
    };

    div()
        .flex()
        .flex_col()
        .rounded(px(13.0))
        .bg(bg)
        .border_1()
        .border_color(border)
        .overflow_hidden()
}

// ── Separators ───────────────────────────────────────────────────────────────

/// Thin 0.5 px separator indented from the left (iOS list-row style).
///
/// The left margin of 52 px aligns with content that sits after a 30 px icon
/// plus padding, matching the standard iOS settings-list layout.
pub fn separator(dark: bool) -> impl IntoElement {
    let color = if dark {
        hsla(0.0, 0.0, 1.0, 0.08)
    } else {
        hsla(0.0, 0.0, 0.0, 0.08)
    };
    div().w_full().h(px(0.5)).bg(color).ml(px(52.0))
}

/// Full-width thin 0.5 px separator.
///
/// Use this between content blocks that don't have a leading icon column.
pub fn separator_full(dark: bool) -> impl IntoElement {
    let color = if dark {
        hsla(0.0, 0.0, 1.0, 0.08)
    } else {
        hsla(0.0, 0.0, 0.0, 0.08)
    };
    div().w_full().h(px(0.5)).bg(color)
}
