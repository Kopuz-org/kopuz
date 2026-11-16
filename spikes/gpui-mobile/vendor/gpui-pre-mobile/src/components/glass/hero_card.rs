//! Glass hero card component.
//!
//! A prominent card with a gradient mesh background and a frosted-glass
//! overlay panel — the centrepiece of the Apple Glass design section.

use gpui::{div, hsla, linear_color_stop, linear_gradient, prelude::*, px};

/// Renders a hero card with a gradient mesh background and glass overlay.
///
/// The card features:
/// - A diagonal linear gradient background (purple → pink tones)
/// - A translucent overlay panel with icon, title, subtitle, and description
/// - Adapts to light and dark mode
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::glass;
///
/// let card = glass::hero_card(true); // dark mode
/// ```
pub fn hero_card(dark: bool) -> impl IntoElement {
    let text_primary = if dark {
        hsla(0.0, 0.0, 1.0, 0.92)
    } else {
        hsla(0.0, 0.0, 0.0, 0.85)
    };
    let text_secondary = if dark {
        hsla(0.0, 0.0, 1.0, 0.55)
    } else {
        hsla(0.0, 0.0, 0.0, 0.45)
    };
    let panel_bg = if dark {
        hsla(0.72, 0.5, 0.3, 0.25)
    } else {
        hsla(0.72, 0.5, 0.85, 0.6)
    };
    let panel_border = if dark {
        hsla(0.72, 0.3, 0.5, 0.2)
    } else {
        hsla(0.72, 0.3, 0.8, 0.3)
    };

    div()
        .flex()
        .flex_col()
        .rounded(px(16.0))
        .overflow_hidden()
        .bg(linear_gradient(
            135.0,
            linear_color_stop(
                if dark {
                    hsla(0.6, 0.6, 0.2, 1.0)
                } else {
                    hsla(0.6, 0.4, 0.85, 1.0)
                },
                0.0,
            ),
            linear_color_stop(
                if dark {
                    hsla(0.8, 0.5, 0.25, 1.0)
                } else {
                    hsla(0.8, 0.4, 0.9, 1.0)
                },
                1.0,
            ),
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_3()
                .p_5()
                .bg(panel_bg)
                .border_1()
                .border_color(panel_border)
                .rounded(px(16.0))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .size(px(44.0))
                                .rounded(px(10.0))
                                .bg(hsla(0.6, 0.7, 0.6, 0.3))
                                .text_xl()
                                .child("🍎"),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .text_lg()
                                        .text_color(text_primary)
                                        .child("Frosted Glass UI"),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(text_secondary)
                                        .child("Translucent panels with vibrancy"),
                                ),
                        ),
                )
                .child(div().text_sm().text_color(text_secondary).child(
                    "Layered translucent materials create depth and \
                             visual hierarchy, inspired by visionOS and iOS design.",
                )),
        )
}
