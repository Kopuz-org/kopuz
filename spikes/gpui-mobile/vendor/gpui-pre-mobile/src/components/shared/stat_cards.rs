//! Shared stat card components — metric cards with title, value, and trend.
//!
//! Stat cards are compact dashboard-style elements that display a key
//! metric with its current value and a trend indicator. Each card features:
//!
//! - A coloured **accent dot** for visual identification
//! - A **title** label (e.g. "Users", "Revenue")
//! - A large **value** (e.g. "1.2k", "$4.8k")
//! - A **trend** string with directional indicator (e.g. "↑ 12%", "↓ 3%")
//!
//! Two public functions are provided:
//!
//! - [`stat_card`] — a single metric card with configurable colours and content
//! - [`stat_cards`] — a composite 2×2 grid of example stat cards (Users,
//!   Revenue, Orders, Rating)

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
const TEAL: u32 = 0x94e2d5;

// ── Single stat card ─────────────────────────────────────────────────────────

/// Renders a single stat card with an accent dot, title, value, and trend.
///
/// The layout:
/// ```text
/// ┌──────────────────────┐
/// │  ● Users             │
/// │  1.2k                │
/// │  ↑ 12%               │
/// └──────────────────────┘
/// ```
///
/// # Parameters
///
/// - `title` — metric label (e.g. "Users", "Revenue")
/// - `value` — current value string (e.g. "1.2k", "$4.8k")
/// - `trend` — trend indicator string (e.g. "↑ 12%", "↓ 3%", "★★★★★")
/// - `accent` — colour of the leading dot indicator (RGB u32)
/// - `trend_color` — colour of the trend text (RGB u32, e.g. green for up, red for down)
/// - `card_bg` — card background colour (RGB u32)
/// - `text_color` — primary text colour for the value (RGB u32)
/// - `sub_text` — secondary text colour for the title (RGB u32)
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::shared;
///
/// let card = shared::stat_card(
///     "Users", "1.2k", "↑ 12%",
///     0x89b4fa, 0xa6e3a1, card_bg, text_color, sub_text,
/// );
/// ```
#[allow(clippy::too_many_arguments)]
pub fn stat_card(
    title: &str,
    value: &str,
    trend: &str,
    accent: u32,
    trend_color: u32,
    card_bg: u32,
    text_color: u32,
    sub_text: u32,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .gap_2()
        .p_4()
        .rounded_xl()
        .bg(rgb(card_bg))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .child(div().size(px(8.0)).rounded_full().bg(rgb(accent)))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(sub_text))
                        .child(title.to_string()),
                ),
        )
        .child(
            div()
                .text_xl()
                .text_color(rgb(text_color))
                .child(value.to_string()),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(trend_color))
                .child(trend.to_string()),
        )
}

// ── Stat cards grid (composite) ──────────────────────────────────────────────

/// Renders a 2×2 grid of example stat cards: Users, Revenue, Orders,
/// and Rating.
///
/// The grid layout:
/// ```text
/// ┌────────────┐  ┌────────────┐
/// │  ● Users   │  │  ● Revenue │
/// │  1.2k      │  │  $4.8k     │
/// │  ↑ 12%     │  │  ↑ 8%      │
/// └────────────┘  └────────────┘
/// ┌────────────┐  ┌────────────┐
/// │  ● Orders  │  │  ● Rating  │
/// │  328       │  │  4.9       │
/// │  ↓ 3%      │  │  ★★★★★    │
/// └────────────┘  └────────────┘
/// ```
///
/// Colours adapt to dark / light mode using the Catppuccin Mocha palette.
/// Upward trends are shown in green, downward in red, and stars in yellow.
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::shared;
///
/// let grid = shared::stat_cards(true); // dark mode
/// ```
pub fn stat_cards(dark: bool) -> impl IntoElement {
    let card_bg = if dark { SURFACE0 } else { 0xe6e9ef };
    let text_color = if dark { TEXT } else { 0x4c4f69 };
    let sub_text: u32 = if dark { 0xa6adc8 } else { 0x6c6f85 };

    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .flex()
                .flex_row()
                .gap_3()
                .child(stat_card(
                    "Users", "1.2k", "↑ 12%", BLUE, GREEN, card_bg, text_color, sub_text,
                ))
                .child(stat_card(
                    "Revenue", "$4.8k", "↑ 8%", MAUVE, GREEN, card_bg, text_color, sub_text,
                )),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .gap_3()
                .child(stat_card(
                    "Orders", "328", "↓ 3%", PEACH, RED, card_bg, text_color, sub_text,
                ))
                .child(stat_card(
                    "Rating",
                    "4.9",
                    "★★★★★",
                    TEAL,
                    YELLOW,
                    card_bg,
                    text_color,
                    sub_text,
                )),
        )
}
