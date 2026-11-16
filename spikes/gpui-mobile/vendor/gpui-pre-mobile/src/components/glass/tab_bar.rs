//! Glass tab bar component.
//!
//! A bottom tab bar inspired by the iOS tab bar — featuring icon + label
//! items with an active highlight colour. The tab bar sits inside a
//! translucent glass container with a top border separator.

use gpui::{div, hsla, prelude::*, px};

/// Renders an iOS-style bottom tab bar with four items.
///
/// The tab bar layout:
/// ```text
/// ┌──────────────────────────────────────┐
/// │  🏠      🔍      ❤️      👤          │
/// │  Home   Search  Favorites Profile    │
/// └──────────────────────────────────────┘
/// ```
///
/// The first item ("Home") is rendered in an active/highlighted state;
/// the remaining three are rendered as inactive. Adapts to dark / light
/// mode with appropriate translucent backgrounds and text colours.
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::glass;
///
/// let bar = glass::tab_bar(true); // dark mode
/// ```
pub fn tab_bar(dark: bool) -> impl IntoElement {
    let bg = if dark {
        hsla(0.0, 0.0, 1.0, 0.05)
    } else {
        hsla(0.0, 0.0, 1.0, 0.7)
    };
    let border = if dark {
        hsla(0.0, 0.0, 1.0, 0.08)
    } else {
        hsla(0.0, 0.0, 0.0, 0.06)
    };
    let active_color = hsla(0.6, 0.8, 0.6, 1.0);
    let inactive_color = if dark {
        hsla(0.0, 0.0, 1.0, 0.35)
    } else {
        hsla(0.0, 0.0, 0.0, 0.35)
    };

    let tab = |icon: &str, label: &str, active: bool| -> gpui::Div {
        let color = if active { active_color } else { inactive_color };
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(2.0))
            .flex_1()
            .child(div().text_xl().text_color(color).child(icon.to_string()))
            .child(
                div()
                    .text_size(px(10.0))
                    .text_color(color)
                    .child(label.to_string()),
            )
    };

    div()
        .flex()
        .flex_row()
        .items_center()
        .py(px(6.0))
        .px_2()
        .rounded(px(16.0))
        .bg(bg)
        .border_t_1()
        .border_color(border)
        .child(tab("🏠", "Home", true))
        .child(tab("🔍", "Search", false))
        .child(tab("❤️", "Favorites", false))
        .child(tab("👤", "Profile", false))
}
