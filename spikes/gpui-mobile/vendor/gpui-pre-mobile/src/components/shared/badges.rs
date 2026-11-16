//! Shared badge components — solid, outline, and dot-indicator variants.
//!
//! Badges are small labelled elements used to convey status, counts, or
//! categories. Three styles are provided:
//!
//! - **Solid** — a filled pill with contrasting text (e.g. "New", "Live", "3")
//! - **Outline** — a bordered pill with coloured text and no fill
//! - **Dot indicator** — a small coloured dot overlaid on an icon to signal
//!   unread notifications or activity
//!
//! Four public functions are provided:
//!
//! - [`badge_solid`] — a single solid badge
//! - [`badge_outline`] — a single outline badge
//! - [`icon_with_badge`] — an icon with a notification dot overlay
//! - [`badges`] — a composite card showcasing all three badge styles

use gpui::{div, prelude::*, px, rgb};

// ── Colour constants (Catppuccin Mocha) ──────────────────────────────────────

const SURFACE0: u32 = 0x313244;
const TEXT: u32 = 0xcdd6f4;
const BLUE: u32 = 0x89b4fa;
const GREEN: u32 = 0xa6e3a1;
const RED: u32 = 0xf38ba8;
const MAUVE: u32 = 0xcba6f7;
const YELLOW: u32 = 0xf9e2af;
const PEACH: u32 = 0xfab387;
const MANTLE: u32 = 0x181825;

// ── Solid badge ──────────────────────────────────────────────────────────────

/// Renders a solid badge — a small filled pill with contrasting text.
///
/// Solid badges are used for high-emphasis status labels like "New",
/// "Live", or numeric counts.
///
/// # Parameters
///
/// - `label` — badge text (keep it short — 1–4 characters is ideal)
/// - `bg` — background colour (RGB u32)
/// - `fg` — text colour (RGB u32)
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::shared;
///
/// let badge = shared::badge_solid("New", 0x89b4fa, 0x181825);
/// ```
pub fn badge_solid(label: &str, bg: u32, fg: u32) -> impl IntoElement {
    div()
        .px_2()
        .py(px(2.0))
        .rounded(px(10.0))
        .bg(rgb(bg))
        .text_xs()
        .text_color(rgb(fg))
        .child(label.to_string())
}

// ── Outline badge ────────────────────────────────────────────────────────────

/// Renders an outline badge — a bordered pill with coloured text and no fill.
///
/// Outline badges are used for medium-emphasis status labels like "Draft",
/// "Active", "Archived", or overflow counts ("99+").
///
/// # Parameters
///
/// - `label` — badge text
/// - `color` — border and text colour (RGB u32)
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::shared;
///
/// let badge = shared::badge_outline("Draft", 0x89b4fa);
/// ```
pub fn badge_outline(label: &str, color: u32) -> impl IntoElement {
    div()
        .px_2()
        .py(px(2.0))
        .rounded(px(10.0))
        .border_1()
        .border_color(rgb(color))
        .text_xs()
        .text_color(rgb(color))
        .child(label.to_string())
}

// ── Icon with dot badge ──────────────────────────────────────────────────────

/// Renders an icon with a small coloured notification dot in the
/// top-right corner.
///
/// The dot is positioned absolutely and has a border ring matching the
/// background colour for a "cut-out" effect — commonly used to indicate
/// unread notifications, messages, or activity on an icon.
///
/// # Parameters
///
/// - `icon` — emoji or short text for the icon (e.g. "🔔", "💬")
/// - `dot_color` — colour of the notification dot (RGB u32)
/// - `dark` — dark-mode flag (controls the icon text colour)
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::shared;
///
/// let bell = shared::icon_with_badge("🔔", 0xf38ba8, true);
/// ```
pub fn icon_with_badge(icon: &str, dot_color: u32, dark: bool) -> impl IntoElement {
    let icon_color: u32 = if dark { TEXT } else { 0x4c4f69 };

    div()
        .relative()
        .child(
            div()
                .text_xl()
                .text_color(rgb(icon_color))
                .child(icon.to_string()),
        )
        .child(
            div()
                .absolute()
                .top_0()
                .right_0()
                .mt(px(-2.0))
                .mr(px(-2.0))
                .size(px(10.0))
                .rounded_full()
                .bg(rgb(dot_color))
                .border_2()
                .border_color(rgb(MANTLE)),
        )
}

// ── Badges showcase (composite) ──────────────────────────────────────────────

/// Renders a card showcasing three badge patterns:
///
/// 1. **Solid badges** — New (blue), Live (green), Error (red), Beta (mauve),
///    and a numeric "3" (peach) badge
/// 2. **Outline badges** — Draft (blue), Active (green), Archived (yellow),
///    and "99+" (red) overflow badge
/// 3. **Dot indicators** — bell, chat, and mail icons each with a coloured
///    notification dot
///
/// The card has a rounded background and adapts to dark / light mode.
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::shared;
///
/// let showcase = shared::badges(true); // dark mode
/// ```
pub fn badges(dark: bool) -> impl IntoElement {
    let card_bg = if dark { SURFACE0 } else { 0xe6e9ef };

    div()
        .flex()
        .flex_col()
        .gap_3()
        .p_4()
        .rounded(px(12.0))
        .bg(rgb(card_bg))
        // Solid badges
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(badge_solid("New", BLUE, MANTLE))
                .child(badge_solid("Live", GREEN, MANTLE))
                .child(badge_solid("Error", RED, MANTLE))
                .child(badge_solid("Beta", MAUVE, MANTLE))
                .child(badge_solid("3", PEACH, MANTLE)),
        )
        // Outline badges
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(badge_outline("Draft", BLUE))
                .child(badge_outline("Active", GREEN))
                .child(badge_outline("Archived", YELLOW))
                .child(badge_outline("99+", RED)),
        )
        // Dot badges (notification indicator)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_4()
                .child(icon_with_badge("🔔", RED, dark))
                .child(icon_with_badge("💬", BLUE, dark))
                .child(icon_with_badge("📧", GREEN, dark)),
        )
}
