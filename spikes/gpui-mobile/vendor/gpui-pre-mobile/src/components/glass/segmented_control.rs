//! Glass segmented control component.
//!
//! An iOS-style multi-segment picker with a translucent track background
//! and a highlighted active segment — commonly used for switching between
//! views or time ranges (e.g. Day / Week / Month / Year).

use gpui::{div, hsla, prelude::*, px};

/// Renders an iOS-style segmented control with four segments.
///
/// The control has a translucent track background with a highlighted
/// active segment that appears elevated. Adapts to dark / light mode.
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::glass;
///
/// let control = glass::segmented_control(true); // dark mode
/// ```
pub fn segmented_control(dark: bool) -> impl IntoElement {
    let track_bg = if dark {
        hsla(0.0, 0.0, 1.0, 0.06)
    } else {
        hsla(0.0, 0.0, 0.0, 0.04)
    };
    let active_bg = if dark {
        hsla(0.0, 0.0, 1.0, 0.12)
    } else {
        hsla(0.0, 0.0, 1.0, 0.85)
    };
    let active_fg = if dark {
        hsla(0.0, 0.0, 1.0, 0.92)
    } else {
        hsla(0.0, 0.0, 0.0, 0.85)
    };
    let inactive_fg = if dark {
        hsla(0.0, 0.0, 1.0, 0.45)
    } else {
        hsla(0.0, 0.0, 0.0, 0.4)
    };
    let border = if dark {
        hsla(0.0, 0.0, 1.0, 0.1)
    } else {
        hsla(0.0, 0.0, 0.0, 0.08)
    };

    let segment = |label: &str, active: bool| -> gpui::Div {
        div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .py(px(7.0))
            .rounded(px(8.0))
            .text_sm()
            .when(active, |d| {
                d.bg(active_bg)
                    .text_color(active_fg)
                    .border_1()
                    .border_color(border)
            })
            .when(!active, |d| d.text_color(inactive_fg))
            .child(label.to_string())
    };

    div()
        .flex()
        .flex_row()
        .p(px(2.0))
        .rounded(px(10.0))
        .bg(track_bg)
        .border_1()
        .border_color(border)
        .child(segment("Day", false))
        .child(segment("Week", true))
        .child(segment("Month", false))
        .child(segment("Year", false))
}
