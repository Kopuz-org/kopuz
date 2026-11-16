//! Shared skeleton loader components — placeholder loading skeletons for
//! cards, text blocks, and image + text layouts.
//!
//! Skeleton loaders are placeholder UI elements that indicate content is
//! loading. They use neutral "bone" coloured rectangles and circles that
//! mimic the shape of the final content, giving users a sense of the
//! layout before data arrives.
//!
//! The composite [`skeleton_loaders`] function renders a card containing
//! three skeleton patterns:
//!
//! 1. **Card skeleton** — a circular avatar placeholder next to two text
//!    lines of different widths
//! 2. **Text block skeleton** — three horizontal bars simulating a
//!    paragraph of text
//! 3. **Image + text skeleton** — a square image placeholder next to
//!    three text lines of varying widths

use gpui::{div, prelude::*, px, relative, rgb};

// ── Colour constants (Catppuccin Mocha) ──────────────────────────────────────

const SURFACE0: u32 = 0x313244;
const SURFACE1: u32 = 0x45475a;

// ── Skeleton loaders card (composite) ────────────────────────────────────────

/// Renders a card containing three skeleton loading patterns: a card
/// skeleton, a text-block skeleton, and an image + text skeleton.
///
/// Each "bone" element uses a neutral surface colour that contrasts
/// subtly with the card background, simulating the shimmer effect
/// found in native loading states.
///
/// The card has a rounded background and adapts to dark / light mode.
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::shared;
///
/// let loaders = shared::skeleton_loaders(true); // dark mode
/// ```
pub fn skeleton_loaders(dark: bool) -> impl IntoElement {
    let card_bg = if dark { SURFACE0 } else { 0xe6e9ef };
    let bone = if dark { SURFACE1 } else { 0xdce0e8 };

    div()
        .flex()
        .flex_col()
        .gap_3()
        .p_4()
        .rounded(px(12.0))
        .bg(rgb(card_bg))
        // Card skeleton — avatar circle + two text lines
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_3()
                .child(div().size(px(40.0)).rounded_full().bg(rgb(bone)))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .gap_2()
                        .child(
                            div()
                                .h(px(14.0))
                                .w(relative(0.6))
                                .rounded(px(4.0))
                                .bg(rgb(bone)),
                        )
                        .child(
                            div()
                                .h(px(10.0))
                                .w(relative(0.4))
                                .rounded(px(4.0))
                                .bg(rgb(bone)),
                        ),
                ),
        )
        // Text block skeleton — three horizontal bars
        .child(div().h(px(12.0)).w_full().rounded(px(4.0)).bg(rgb(bone)))
        .child(div().h(px(12.0)).w_full().rounded(px(4.0)).bg(rgb(bone)))
        .child(
            div()
                .h(px(12.0))
                .w(relative(0.75))
                .rounded(px(4.0))
                .bg(rgb(bone)),
        )
        // Image + text skeleton — square image placeholder + three text lines
        .child(
            div()
                .flex()
                .flex_row()
                .gap_3()
                .mt_2()
                .child(div().size(px(80.0)).rounded(px(8.0)).bg(rgb(bone)))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .gap_2()
                        .justify_center()
                        .child(
                            div()
                                .h(px(14.0))
                                .w(relative(0.8))
                                .rounded(px(4.0))
                                .bg(rgb(bone)),
                        )
                        .child(
                            div()
                                .h(px(10.0))
                                .w(relative(0.5))
                                .rounded(px(4.0))
                                .bg(rgb(bone)),
                        )
                        .child(
                            div()
                                .h(px(10.0))
                                .w(relative(0.65))
                                .rounded(px(4.0))
                                .bg(rgb(bone)),
                        ),
                ),
        )
}
