//! Material card components — elevated and outlined variants.
//!
//! Cards contain content and actions about a single subject. Material
//! Design 3 defines several card types:
//!
//! - **Elevated** — a card with a subtle shadow/border for visual hierarchy,
//!   typically containing an avatar, title, body text, and action buttons.
//! - **Outlined** — a card with a visible border instead of elevation,
//!   best for grouping related content without heavy emphasis.
//!
//! The composite [`cards`] function renders both variants in a vertical
//! stack for showcase purposes. Individual card styles can be built using
//! the [`super::surface::surface`] base and the button primitives.

use gpui::{div, prelude::*, px, rgb};

use super::buttons::{button_filled, button_text};
use super::surface::surface;

/// Renders a complete Material Design cards showcase with two variants:
///
/// 1. **Elevated card** — avatar, title, subtitle, body text, and action
///    buttons (Cancel + Accept) inside an elevation-2 surface.
/// 2. **Outlined card** — title and body text inside a bordered container
///    with no elevation.
///
/// Colours adapt to dark / light mode using the MD3 colour system.
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::material;
///
/// let showcase = material::cards(true); // dark mode
/// ```
pub fn cards(dark: bool) -> impl IntoElement {
    let on_surface = if dark { 0xe6e1e5_u32 } else { 0x1c1b1f };
    let on_surface_variant = if dark { 0xcac4d0_u32 } else { 0x49454f };
    let primary = if dark { 0xd0bcff_u32 } else { 0x6750a4 };

    div()
        .flex()
        .flex_col()
        .gap_3()
        // Elevated card
        .child(
            surface(dark, 2).child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_4()
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
                                    .size(px(40.0))
                                    .rounded_full()
                                    .bg(rgb(primary))
                                    .text_color(rgb(if dark { 0x381e72 } else { 0xffffff }))
                                    .text_sm()
                                    .child("AB"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .text_base()
                                            .text_color(rgb(on_surface))
                                            .child("Elevated Card"),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(on_surface_variant))
                                            .child("Material Design 3"),
                                    ),
                            ),
                    )
                    .child(div().text_sm().text_color(rgb(on_surface_variant)).child(
                        "Cards contain content and actions about a single subject. \
                                 Elevated cards have a drop shadow for visual hierarchy.",
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_end()
                            .gap_2()
                            .child(button_text("Cancel", primary))
                            .child(button_filled(
                                "Accept",
                                primary,
                                if dark { 0x381e72 } else { 0xffffff },
                            )),
                    ),
            ),
        )
        // Outlined card
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .p_4()
                .rounded(px(12.0))
                .border_1()
                .border_color(rgb(if dark { 0x49454f } else { 0xc4c0c9 }))
                .bg(rgb(if dark { 0x1c1b1f } else { 0xffffff }))
                .child(
                    div()
                        .text_base()
                        .text_color(rgb(on_surface))
                        .child("Outlined Card"),
                )
                .child(div().text_sm().text_color(rgb(on_surface_variant)).child(
                    "Outlined cards use a border instead of shadow. \
                             Best for grouping related content without heavy emphasis.",
                )),
        )
}
