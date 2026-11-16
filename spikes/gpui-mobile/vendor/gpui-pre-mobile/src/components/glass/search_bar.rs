//! Glass search bar component.
//!
//! A rounded search input field inspired by the iOS search bar — featuring
//! a magnifying glass icon, placeholder text, and a circular clear button.
//! Adapts to light and dark mode with appropriate translucent backgrounds.

use gpui::{div, hsla, prelude::*, px};

/// Renders an iOS-style search bar with icon, placeholder, and clear button.
///
/// The search bar layout:
/// ```text
/// ┌──────────────────────────────────────┐
/// │  🔍  Search…                     ✕   │
/// └──────────────────────────────────────┘
/// ```
///
/// This is a purely visual component — it does not handle text input or
/// state. Use it as a placeholder or combine it with GPUI's text input
/// primitives for interactivity.
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::glass;
///
/// let bar = glass::search_bar(true); // dark mode
/// ```
pub fn search_bar(dark: bool) -> impl IntoElement {
    let bg = if dark {
        hsla(0.0, 0.0, 1.0, 0.06)
    } else {
        hsla(0.0, 0.0, 0.0, 0.04)
    };
    let border = if dark {
        hsla(0.0, 0.0, 1.0, 0.08)
    } else {
        hsla(0.0, 0.0, 0.0, 0.06)
    };
    let placeholder = if dark {
        hsla(0.0, 0.0, 1.0, 0.3)
    } else {
        hsla(0.0, 0.0, 0.0, 0.3)
    };

    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .px_3()
        .py(px(10.0))
        .rounded(px(12.0))
        .bg(bg)
        .border_1()
        .border_color(border)
        .child(div().text_base().text_color(placeholder).child("🔍"))
        .child(
            div()
                .flex_1()
                .text_base()
                .text_color(placeholder)
                .child("Search…"),
        )
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(20.0))
                .rounded_full()
                .bg(if dark {
                    hsla(0.0, 0.0, 1.0, 0.1)
                } else {
                    hsla(0.0, 0.0, 0.0, 0.08)
                })
                .text_xs()
                .text_color(placeholder)
                .child("✕"),
        )
}
