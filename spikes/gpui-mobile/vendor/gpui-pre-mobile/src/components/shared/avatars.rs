//! Shared avatar components — circles with initials, status indicators,
//! and stacked groups.
//!
//! Avatars are circular elements displaying user initials with a coloured
//! background. They come in several variants:
//!
//! - **Basic** — a single circle with initials, configurable size and colours
//! - **With status** — an avatar with a small coloured dot indicator
//!   (online, away, busy, offline) and a status label below
//! - **Stacked group** — overlapping avatars representing a group of users
//!
//! Three public functions are provided:
//!
//! - [`avatar`] — a single avatar circle with initials
//! - [`avatar_status`] — an avatar with a status dot and label
//! - [`avatars`] — a composite card showcasing size variants, status
//!   indicators, and a stacked group

use gpui::{div, prelude::*, px, rgb, Pixels};

// ── Colour constants (Catppuccin Mocha) ──────────────────────────────────────

const SURFACE0: u32 = 0x313244;
const TEXT: u32 = 0xcdd6f4;
const BLUE: u32 = 0x89b4fa;
const GREEN: u32 = 0xa6e3a1;
const RED: u32 = 0xf38ba8;
const MAUVE: u32 = 0xcba6f7;
const YELLOW: u32 = 0xf9e2af;
const PEACH: u32 = 0xfab387;
const TEAL: u32 = 0x94e2d5;
const MANTLE: u32 = 0x181825;
const SURFACE1: u32 = 0x45475a;

// ── Single avatar ────────────────────────────────────────────────────────────

/// Renders a single avatar circle with initials.
///
/// The avatar is a circular element with a coloured background, centred
/// initials text, and a thin border. The text size scales proportionally
/// to the avatar size (40 % of the diameter).
///
/// # Parameters
///
/// - `initials` — one or two characters displayed in the centre
/// - `bg` — background colour (RGB u32)
/// - `fg` — text colour (RGB u32)
/// - `size` — diameter of the circle in pixels
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::shared;
///
/// let a = shared::avatar("AB", 0x89b4fa, 0x181825, px(40.0));
/// ```
pub fn avatar(initials: &str, bg: u32, fg: u32, size: Pixels) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .justify_center()
        .size(size)
        .rounded_full()
        .bg(rgb(bg))
        .text_color(rgb(fg))
        .text_size(size * 0.4)
        .border_2()
        .border_color(rgb(MANTLE))
        .child(initials.to_string())
}

// ── Avatar with status indicator ─────────────────────────────────────────────

/// Renders an avatar with a small coloured status-dot indicator and a
/// label below.
///
/// The status dot is positioned at the bottom-right corner of the avatar
/// and has a border ring matching the background colour for a "cut-out"
/// effect.
///
/// # Parameters
///
/// - `initials` — one or two characters displayed in the avatar
/// - `bg` — avatar background colour (RGB u32)
/// - `status_color` — colour of the status dot (e.g. green for online)
/// - `status_label` — text shown below the avatar (e.g. "Online", "Away")
/// - `label_color` — colour for the status label text (RGB u32)
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::shared;
///
/// let online = shared::avatar_status("JD", 0x89b4fa, 0xa6e3a1, "Online", 0xa6adc8);
/// ```
pub fn avatar_status(
    initials: &str,
    bg: u32,
    status_color: u32,
    status_label: &str,
    label_color: u32,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap_1()
        .child(
            div()
                .relative()
                .child(avatar(initials, bg, MANTLE, px(40.0)))
                .child(
                    div()
                        .absolute()
                        .bottom_0()
                        .right_0()
                        .size(px(12.0))
                        .rounded_full()
                        .bg(rgb(status_color))
                        .border_2()
                        .border_color(rgb(MANTLE)),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(label_color))
                .child(status_label.to_string()),
        )
}

// ── Avatars showcase (composite) ─────────────────────────────────────────────

/// Renders a card showcasing three avatar patterns:
///
/// 1. **Size variants** — five avatars in decreasing sizes (48 → 28 px)
///    with different colours and initials
/// 2. **Status indicators** — four avatars with online (green), away
///    (yellow), busy (red), and offline (gray) status dots
/// 3. **Stacked group** — five overlapping avatars with a "+3" overflow
///    indicator, simulating a group/team display
///
/// The card has a rounded background and adapts to dark / light mode.
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::shared;
///
/// let showcase = shared::avatars(true); // dark mode
/// ```
pub fn avatars(dark: bool) -> impl IntoElement {
    let card_bg = if dark { SURFACE0 } else { 0xe6e9ef };
    let sub_text: u32 = if dark { 0xa6adc8 } else { 0x6c6f85 };

    div()
        .flex()
        .flex_col()
        .gap_3()
        .p_4()
        .rounded(px(12.0))
        .bg(rgb(card_bg))
        // Size variants
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_3()
                .child(avatar("A", BLUE, MANTLE, px(48.0)))
                .child(avatar("BP", GREEN, MANTLE, px(40.0)))
                .child(avatar("ZD", MAUVE, MANTLE, px(36.0)))
                .child(avatar("M", PEACH, MANTLE, px(32.0)))
                .child(avatar("K", TEAL, MANTLE, px(28.0))),
        )
        // With status indicators
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_4()
                .child(avatar_status("JD", BLUE, GREEN, "Online", sub_text))
                .child(avatar_status("AR", MAUVE, YELLOW, "Away", sub_text))
                .child(avatar_status("TX", PEACH, RED, "Busy", sub_text))
                .child(avatar_status("KL", TEAL, SURFACE1, "Offline", sub_text)),
        )
        // Stacked / group avatar
        .child(
            div()
                .flex()
                .flex_row()
                .child(avatar("A", BLUE, MANTLE, px(36.0)))
                .child(
                    div()
                        .ml(px(-10.0))
                        .child(avatar("B", GREEN, MANTLE, px(36.0))),
                )
                .child(
                    div()
                        .ml(px(-10.0))
                        .child(avatar("C", MAUVE, MANTLE, px(36.0))),
                )
                .child(
                    div()
                        .ml(px(-10.0))
                        .child(avatar("D", PEACH, MANTLE, px(36.0))),
                )
                .child(
                    div()
                        .ml(px(-10.0))
                        .child(avatar("+3", SURFACE1, TEXT, px(36.0))),
                ),
        )
}
