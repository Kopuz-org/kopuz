//! Material floating action button (FAB) components.
//!
//! Floating action buttons represent the primary action of a screen.
//! Material Design 3 defines four FAB sizes:
//!
//! - **Small** — 40 × 40 px, 12 px corner radius
//! - **Regular** — 56 × 56 px, 16 px corner radius
//! - **Large** — 96 × 96 px, 28 px corner radius
//! - **Extended** — 56 px tall, variable width, icon + label
//!
//! The composite [`fabs`] function renders all four variants in a single
//! row for showcase purposes. You can also build individual FABs using
//! the MD3 colour tokens and the `material::surface` base.

use gpui::{div, prelude::*, px, rgb};

/// Renders a showcase row of all four FAB sizes: small, regular, large,
/// and extended.
///
/// Colours adapt to dark / light mode using the MD3 colour system:
/// - Small FAB uses the secondary container colour
/// - Regular FAB uses the primary container colour
/// - Large FAB uses the tertiary container colour
/// - Extended FAB uses the primary container colour with an icon + label
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::material;
///
/// let fab_row = material::fabs(true); // dark mode
/// ```
pub fn fabs(dark: bool) -> impl IntoElement {
    let md_primary_container = if dark { 0x4f378b_u32 } else { 0xeaddff };
    let md_on_primary_container = if dark { 0xeaddff_u32 } else { 0x21005e };
    let md_secondary_container = if dark { 0x4a4458_u32 } else { 0xe8def8 };
    let md_on_secondary_container = if dark { 0xe8def8_u32 } else { 0x1d192b };
    let md_tertiary_container = if dark { 0x633b48_u32 } else { 0xffd8e4 };
    let md_on_tertiary_container = if dark { 0xffd8e4_u32 } else { 0x31111d };

    div()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_end()
        .gap_3()
        // Small FAB
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(40.0))
                .rounded(px(12.0))
                .bg(rgb(md_secondary_container))
                .text_lg()
                .text_color(rgb(md_on_secondary_container))
                .child("+"),
        )
        // Regular FAB
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(56.0))
                .rounded(px(16.0))
                .bg(rgb(md_primary_container))
                .text_2xl()
                .text_color(rgb(md_on_primary_container))
                .child("✏️"),
        )
        // Large FAB
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(96.0))
                .rounded(px(28.0))
                .bg(rgb(md_tertiary_container))
                .text_3xl()
                .text_color(rgb(md_on_tertiary_container))
                .child("📷"),
        )
        // Extended FAB
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .px_4()
                .h(px(56.0))
                .rounded(px(16.0))
                .bg(rgb(md_primary_container))
                .text_color(rgb(md_on_primary_container))
                .child(div().text_lg().child("✏️"))
                .child(div().text_sm().child("Compose")),
        )
}
