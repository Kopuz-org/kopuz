//! Material bottom sheet component — modal sheet with drag handle and items.
//!
//! Bottom sheets are surfaces containing supplementary content that are
//! anchored to the bottom of the screen. Material Design 3 bottom sheets
//! feature:
//!
//! - A **drag handle** at the top for visual affordance
//! - A **title** describing the sheet's purpose
//! - A list of **action items**, each with an icon and label
//!
//! Two public functions are provided:
//!
//! - [`sheet_item`] — a single action row with an icon and label
//! - [`bottom_sheet`] — a composite sheet with a drag handle, title, and
//!   four example share actions (Email, Messages, Copy Link, AirDrop)

use gpui::{div, prelude::*, px, rgb};

// ── Single sheet item ────────────────────────────────────────────────────────

/// Renders a single bottom sheet action row with an icon and label.
///
/// The layout:
/// ```text
/// │  📧  Email                              │
/// ```
///
/// # Parameters
///
/// - `icon` — emoji or short text for the leading icon
/// - `label` — action text
/// - `on_surface` — primary text colour (RGB u32)
/// - `on_surface_variant` — icon colour (RGB u32)
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::material;
///
/// let item = material::sheet_item("📧", "Email", 0x1c1b1f, 0x49454f);
/// ```
pub fn sheet_item(
    icon: &str,
    label: &str,
    on_surface: u32,
    on_surface_variant: u32,
) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_4()
        .px_6()
        .py_3()
        .child(
            div()
                .text_xl()
                .text_color(rgb(on_surface_variant))
                .child(icon.to_string()),
        )
        .child(
            div()
                .text_base()
                .text_color(rgb(on_surface))
                .child(label.to_string()),
        )
}

// ── Bottom sheet (composite) ─────────────────────────────────────────────────

/// Renders a Material Design modal bottom sheet with a drag handle, title,
/// and four share-action items.
///
/// The sheet layout:
/// ```text
/// ╭──────────────────────────────────────╮
/// │            ━━━━━━━━━━                │  ← drag handle
/// │  Share with…                         │  ← title
/// │  📧  Email                           │
/// │  💬  Messages                        │
/// │  📋  Copy Link                       │
/// │  📱  AirDrop                         │
/// ╰──────────────────────────────────────╯
/// ```
///
/// The top corners use a large 28 px radius (MD3 standard for bottom
/// sheets) while the bottom corners use a smaller 12 px radius since
/// this is rendered inline rather than anchored to the screen edge.
///
/// Colours adapt to dark / light mode using the MD3 colour system.
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::material;
///
/// let sheet = material::bottom_sheet(true); // dark mode
/// ```
pub fn bottom_sheet(dark: bool) -> impl IntoElement {
    let surface = if dark { 0x2b2930_u32 } else { 0xf3edf7 };
    let on_surface = if dark { 0xe6e1e5_u32 } else { 0x1c1b1f };
    let on_surface_variant = if dark { 0xcac4d0_u32 } else { 0x49454f };
    let drag_handle = if dark { 0x49454f_u32 } else { 0xc4c0c9 };

    div()
        .flex()
        .flex_col()
        .rounded_tl(px(28.0))
        .rounded_tr(px(28.0))
        .rounded_bl(px(12.0))
        .rounded_br(px(12.0))
        .bg(rgb(surface))
        .overflow_hidden()
        // Drag handle
        .child(
            div().flex().items_center().justify_center().py_3().child(
                div()
                    .w(px(32.0))
                    .h(px(4.0))
                    .rounded(px(2.0))
                    .bg(rgb(drag_handle)),
            ),
        )
        // Title
        .child(
            div()
                .px_6()
                .pb_2()
                .text_base()
                .text_color(rgb(on_surface))
                .child("Share with…"),
        )
        // Sheet items
        .child(sheet_item("📧", "Email", on_surface, on_surface_variant))
        .child(sheet_item("💬", "Messages", on_surface, on_surface_variant))
        .child(sheet_item(
            "📋",
            "Copy Link",
            on_surface,
            on_surface_variant,
        ))
        .child(sheet_item("📱", "AirDrop", on_surface, on_surface_variant))
        .child(div().h(px(8.0)))
}
