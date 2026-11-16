//! Material hero card component.
//!
//! A prominent card with a diagonal gradient background in green tones
//! and white text — the centrepiece of the Material Design section.
//! Features an icon, title, subtitle, and description text.

use gpui::{div, hsla, linear_color_stop, linear_gradient, prelude::*, px};

/// Renders a Material Design hero card with a gradient background.
///
/// The card features:
/// - A diagonal linear gradient background (green tones)
/// - A circular icon container with a translucent white background
/// - Title, subtitle, and description text in white/near-white
/// - Adapts green intensity to dark / light mode
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::material;
///
/// let card = material::hero_card(true); // dark mode
/// ```
pub fn hero_card(dark: bool) -> impl IntoElement {
    let text_on_primary = hsla(0.0, 0.0, 1.0, 0.95);

    div()
        .flex()
        .flex_col()
        .rounded(px(16.0))
        .overflow_hidden()
        .bg(linear_gradient(
            135.0,
            linear_color_stop(
                if dark {
                    hsla(0.38, 0.65, 0.35, 1.0)
                } else {
                    hsla(0.38, 0.65, 0.42, 1.0)
                },
                0.0,
            ),
            linear_color_stop(
                if dark {
                    hsla(0.45, 0.55, 0.3, 1.0)
                } else {
                    hsla(0.45, 0.55, 0.38, 1.0)
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
                                .rounded_full()
                                .bg(hsla(0.0, 0.0, 1.0, 0.15))
                                .text_xl()
                                .child("🤖"),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .text_lg()
                                        .text_color(text_on_primary)
                                        .child("Material Design"),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(hsla(0.0, 0.0, 1.0, 0.7))
                                        .child("Elevation · Color · Motion"),
                                ),
                        ),
                )
                .child(div().text_sm().text_color(hsla(0.0, 0.0, 1.0, 0.75)).child(
                    "Material Design uses environmental cues like \
                             surfaces, depth, and shadow to express hierarchy.",
                )),
        )
}
