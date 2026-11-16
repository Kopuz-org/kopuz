//! Material chip components — assist, filter, and suggestion chips.
//!
//! Chips are compact elements that represent an input, attribute, or action.
//! Material Design 3 defines several chip types:
//!
//! - **Assist chips** — guide the user towards a related action
//! - **Filter chips** — narrow content using tags or descriptive words
//! - **Suggestion chips** — dynamically generated shortcuts
//!
//! Two public functions are provided:
//!
//! - [`chip`] — a single chip element, optionally in a selected state
//! - [`chips`] — a composite showcase with assist/suggestion and filter rows

use gpui::{div, prelude::*, px, rgb};

// ── Single chip ──────────────────────────────────────────────────────────────

/// Renders a single Material Design chip.
///
/// When `selected` is `true`, the chip has a filled background using
/// `selected_bg`. When `false`, it renders with an outlined border.
///
/// # Parameters
///
/// - `label` — chip text (may include a leading icon emoji, e.g. "🎵 Music")
/// - `selected` — whether the chip is in the selected/active state
/// - `outline` — border colour for unselected state (RGB u32)
/// - `on_surface` — text colour (RGB u32)
/// - `selected_bg` — background colour for selected state (RGB u32)
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::material;
///
/// let active = material::chip("✓ Nearby", true, outline, on_surface, selected_bg);
/// let inactive = material::chip("Open Now", false, outline, on_surface, selected_bg);
/// ```
pub fn chip(
    label: &str,
    selected: bool,
    outline: u32,
    on_surface: u32,
    selected_bg: u32,
) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .h(px(32.0))
        .px_3()
        .rounded(px(8.0))
        .when(selected, |d| d.bg(rgb(selected_bg)))
        .when(!selected, |d| d.border_1().border_color(rgb(outline)))
        .text_sm()
        .text_color(rgb(on_surface))
        .child(label.to_string())
}

// ── Chips showcase (composite) ───────────────────────────────────────────────

/// Renders a complete Material Design chips showcase with two rows:
///
/// 1. **Assist / suggestion chips** — Music, Photos (selected), Videos, Docs
/// 2. **Filter chips** — Nearby (selected), Open Now, 4+ Stars (selected), Free WiFi
///
/// Colours adapt to dark / light mode using the MD3 colour system.
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::material;
///
/// let showcase = material::chips(true); // dark mode
/// ```
pub fn chips(dark: bool) -> impl IntoElement {
    let outline = if dark { 0x938f99_u32 } else { 0x79747e };
    let on_surface = if dark { 0xe6e1e5_u32 } else { 0x1c1b1f };
    let selected_bg = if dark { 0x4a4458_u32 } else { 0xe8def8 };

    div()
        .flex()
        .flex_col()
        .gap_3()
        // Assist / suggestion chips
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(chip("🎵 Music", false, outline, on_surface, selected_bg))
                .child(chip("📸 Photos", true, outline, on_surface, selected_bg))
                .child(chip("🎬 Videos", false, outline, on_surface, selected_bg))
                .child(chip("📄 Docs", false, outline, on_surface, selected_bg)),
        )
        // Filter chips
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(chip("✓ Nearby", true, outline, on_surface, selected_bg))
                .child(chip("Open Now", false, outline, on_surface, selected_bg))
                .child(chip("✓ 4+ Stars", true, outline, on_surface, selected_bg))
                .child(chip("Free WiFi", false, outline, on_surface, selected_bg)),
        )
}
