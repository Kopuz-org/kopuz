//! Material snackbar components — single-line and multi-line variants.
//!
//! Snackbars provide brief messages about app processes at the bottom of
//! the screen. Material Design 3 defines two main layouts:
//!
//! - **Single-line** — a short message with an optional action button
//! - **Multi-line** — a longer message that wraps, with action buttons
//!   aligned to the bottom-right
//!
//! The composite [`snackbar`] function renders both variants stacked
//! vertically for showcase purposes.

use gpui::{div, prelude::*, px, rgb};

/// Renders a complete Material Design snackbar showcase with two variants:
///
/// 1. **Single-line** — "File has been deleted" with an UNDO action
/// 2. **Multi-line** — "Connection lost…" message with RETRY and DISMISS actions
///
/// Snackbars use the inverse surface colour scheme (dark bg in light mode,
/// slightly lighter bg in dark mode) to stand out from the main content.
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::material;
///
/// let bars = material::snackbar(true); // dark mode
/// ```
pub fn snackbar(dark: bool) -> impl IntoElement {
    let snack_bg = if dark { 0x332d41_u32 } else { 0x322f35 };
    let snack_text = if dark { 0xe6e1e5_u32 } else { 0xf4eff4 };
    let action_color = 0xd0bcff_u32;

    div()
        .flex()
        .flex_col()
        .gap_2()
        // Single-line snackbar
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .px_4()
                .py_3()
                .rounded(px(4.0))
                .bg(rgb(snack_bg))
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(snack_text))
                        .child("File has been deleted"),
                )
                .child(div().text_sm().text_color(rgb(action_color)).child("UNDO")),
        )
        // Multi-line snackbar
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .px_4()
                .py_3()
                .rounded(px(4.0))
                .bg(rgb(snack_bg))
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(snack_text))
                        .child("Connection lost. Changes will be synced when you're back online."),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_end()
                        .gap_2()
                        .child(div().text_sm().text_color(rgb(action_color)).child("RETRY"))
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(action_color))
                                .child("DISMISS"),
                        ),
                ),
        )
}
