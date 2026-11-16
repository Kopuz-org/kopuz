//! Common layout helpers shared across all design-language modules.
//!
//! These small building blocks are used by the glass, material, and shared
//! component modules to create consistent section headings and labels.

use gpui::{div, prelude::*, px, rgb};

// ── Section / header helpers ─────────────────────────────────────────────────

/// Renders a design-language section header with a coloured accent bar,
/// a title, and a subtitle.
///
/// ```text
/// ┃  Apple Glass
/// ┃  Frosted panels · Vibrancy · SF-style controls
/// ```
pub fn design_language_header(
    title: &str,
    subtitle: &str,
    accent: u32,
    sub_text: u32,
) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_3()
        .child(
            div()
                .w(px(4.0))
                .h(px(32.0))
                .rounded(px(2.0))
                .bg(rgb(accent)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_lg()
                        .text_color(rgb(accent))
                        .child(title.to_string()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(sub_text))
                        .child(subtitle.to_string()),
                ),
        )
}

/// Renders a small uppercase section label.
///
/// Used above groups of components to identify what they are
/// (e.g. "BUTTONS", "SLIDERS", "CHIPS").
pub fn section_label(title: &str, color: u32) -> impl IntoElement {
    div()
        .text_xs()
        .text_color(rgb(color))
        .px_1()
        .child(title.to_string().to_uppercase())
}
