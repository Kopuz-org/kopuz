//! Material button components — filled, tonal, outlined, and text variants.
//!
//! Four button styles following the Material Design 3 specification:
//!
//! - **Filled** — solid background with contrasting text (primary actions)
//! - **Tonal** — muted background with harmonious text (secondary actions)
//! - **Outlined** — transparent with a coloured border (tertiary actions)
//! - **Text** — no background or border, just coloured text (low-emphasis)
//!
//! Each style is available as an individual function, and a composite
//! [`buttons`] function renders a showcase of all four styles in rows.

use gpui::{div, prelude::*, px, rgb};

// ── Filled button ────────────────────────────────────────────────────────────

/// Renders a Material Design filled button.
///
/// A pill-shaped button with a solid `bg` colour and contrasting `fg` text.
/// Used for primary, high-emphasis actions.
///
/// # Parameters
///
/// - `label` — button text
/// - `bg` — background colour (RGB u32)
/// - `fg` — text colour (RGB u32)
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::material;
///
/// let btn = material::button_filled("Accept", 0x6750a4, 0xffffff);
/// ```
pub fn button_filled(label: &str, bg: u32, fg: u32) -> impl IntoElement {
    div()
        .px_6()
        .py(px(10.0))
        .rounded(px(20.0))
        .bg(rgb(bg))
        .text_sm()
        .text_color(rgb(fg))
        .child(label.to_string())
}

// ── Tonal button ─────────────────────────────────────────────────────────────

/// Renders a Material Design tonal (filled-tonal) button.
///
/// A pill-shaped button with a muted background and harmonious text colour.
/// Used for secondary actions that need more emphasis than outlined or text
/// buttons but less than filled buttons.
///
/// # Parameters
///
/// - `label` — button text
/// - `bg` — background colour (RGB u32)
/// - `fg` — text colour (RGB u32)
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::material;
///
/// let btn = material::button_tonal("Tonal", 0xe8def8, 0x1d192b);
/// ```
pub fn button_tonal(label: &str, bg: u32, fg: u32) -> impl IntoElement {
    div()
        .px_6()
        .py(px(10.0))
        .rounded(px(20.0))
        .bg(rgb(bg))
        .text_sm()
        .text_color(rgb(fg))
        .child(label.to_string())
}

// ── Outlined button ──────────────────────────────────────────────────────────

/// Renders a Material Design outlined button.
///
/// A pill-shaped button with a transparent background and a coloured border.
/// Used for medium-emphasis actions or as a secondary option alongside a
/// filled button.
///
/// # Parameters
///
/// - `label` — button text
/// - `outline` — border and text colour (RGB u32)
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::material;
///
/// let btn = material::button_outlined("Cancel", 0x79747e);
/// ```
pub fn button_outlined(label: &str, outline: u32) -> impl IntoElement {
    div()
        .px_6()
        .py(px(10.0))
        .rounded(px(20.0))
        .border_1()
        .border_color(rgb(outline))
        .text_sm()
        .text_color(rgb(outline))
        .child(label.to_string())
}

// ── Text button ──────────────────────────────────────────────────────────────

/// Renders a Material Design text button.
///
/// A button with no background or border — just coloured text. Used for
/// the lowest-emphasis actions, often paired with a higher-emphasis button
/// (e.g. "Cancel" next to "Accept").
///
/// # Parameters
///
/// - `label` — button text
/// - `color` — text colour (RGB u32)
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::material;
///
/// let btn = material::button_text("Learn More", 0x6750a4);
/// ```
pub fn button_text(label: &str, color: u32) -> impl IntoElement {
    div()
        .px_3()
        .py(px(10.0))
        .text_sm()
        .text_color(rgb(color))
        .child(label.to_string())
}

// ── Buttons showcase (composite) ─────────────────────────────────────────────

/// Renders a complete Material Design buttons showcase with four rows:
///
/// 1. **Filled** — primary, accept, and delete buttons
/// 2. **Tonal** — tonal and secondary buttons
/// 3. **Outlined** — outlined and cancel buttons
/// 4. **Text** — text button and "Learn More" link
///
/// Colours adapt to dark / light mode using the MD3 colour system.
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::material;
///
/// let showcase = material::buttons(true); // dark mode
/// ```
pub fn buttons(dark: bool) -> impl IntoElement {
    let md_primary = if dark { 0xd0bcff_u32 } else { 0x6750a4 };
    let md_on_primary = if dark { 0x381e72_u32 } else { 0xffffff };
    let md_secondary = if dark { 0x332d41_u32 } else { 0xe8def8 };
    let md_on_secondary = if dark { 0xe8def8_u32 } else { 0x1d192b };
    let md_outline = if dark { 0x938f99_u32 } else { 0x79747e };
    let md_on_surface = if dark { 0xe6e1e5_u32 } else { 0x1c1b1f };
    let md_surface_variant = if dark { 0x49454f_u32 } else { 0xe7e0ec };

    div()
        .flex()
        .flex_col()
        .gap_3()
        // Row 1: Filled buttons
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(button_filled("Filled", md_primary, md_on_primary))
                .child(button_filled("Accept", 0xa6e3a1, 0x1e1e2e))
                .child(button_filled("Delete", 0xf38ba8, 0x1e1e2e)),
        )
        // Row 2: Tonal buttons
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(button_tonal("Tonal", md_secondary, md_on_secondary))
                .child(button_tonal("Secondary", md_surface_variant, md_on_surface)),
        )
        // Row 3: Outlined buttons
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(button_outlined("Outlined", md_outline))
                .child(button_outlined("Cancel", md_outline)),
        )
        // Row 4: Text buttons
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(button_text("Text Button", md_primary))
                .child(button_text("Learn More", md_primary)),
        )
}
