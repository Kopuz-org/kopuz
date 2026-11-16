//! Glass slider components — labelled slider tracks with thumbs.
//!
//! A set of slider controls inspired by the iOS Control Centre sliders.
//! Each slider row displays a label with an icon, a percentage value,
//! and a track with a coloured fill and a draggable thumb indicator.
//!
//! Two public functions are provided:
//!
//! - [`slider_row`] — a single labelled slider with configurable value and colour
//! - [`sliders`] — a composite panel containing Brightness, Volume, and Opacity sliders

use gpui::{div, hsla, prelude::*, px, relative, rgb};

use super::panel::{panel, separator_full};

// ── Colour constants ─────────────────────────────────────────────────────────

/// Catppuccin Mocha blue.
const BLUE: u32 = 0x89b4fa;
/// Catppuccin Mocha green.
const GREEN: u32 = 0xa6e3a1;
/// Catppuccin Mocha mauve.
const MAUVE: u32 = 0xcba6f7;

// ── Single slider row ────────────────────────────────────────────────────────

/// Renders a single labelled slider row.
///
/// The layout:
/// ```text
/// ☀️ Brightness                  70%
/// ┌──────────────────────────────────┐
/// │ ████████████████████░░░░░░░░ ◉   │
/// └──────────────────────────────────┘
/// ```
///
/// # Parameters
///
/// - `label` — descriptive text (e.g. "Brightness")
/// - `icon` — emoji or short text shown before the label
/// - `value` — fill ratio in the range `0.0..=1.0`
/// - `accent` — RGB colour for the active track fill
/// - `track_inactive` — HSLA colour for the inactive portion of the track
/// - `dark` — dark-mode flag
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::glass;
///
/// let slider = glass::slider_row(
///     "Volume", "🔊", 0.45, 0xa6e3a1,
///     track_inactive_color, true,
/// );
/// ```
pub fn slider_row(
    label: &str,
    icon: &str,
    value: f32,
    accent: u32,
    track_inactive: gpui::Hsla,
    dark: bool,
) -> impl IntoElement {
    let text_secondary = if dark {
        hsla(0.0, 0.0, 1.0, 0.55)
    } else {
        hsla(0.0, 0.0, 0.0, 0.45)
    };

    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .flex()
                .flex_row()
                .justify_between()
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .items_center()
                        .child(div().text_base().child(icon.to_string()))
                        .child(
                            div()
                                .text_sm()
                                .text_color(text_secondary)
                                .child(label.to_string()),
                        ),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(text_secondary)
                        .child(format!("{}%", (value * 100.0) as u32)),
                ),
        )
        .child(
            // Track
            div()
                .w_full()
                .h(px(6.0))
                .rounded(px(3.0))
                .bg(track_inactive)
                .relative()
                .child(
                    // Fill
                    div()
                        .h_full()
                        .rounded(px(3.0))
                        .bg(rgb(accent))
                        .w(relative(value)),
                )
                // Thumb (overlaid at the end of the fill)
                .child(
                    div()
                        .absolute()
                        .top(px(-9.0))
                        .left(relative(value))
                        .ml(px(-12.0))
                        .size(px(24.0))
                        .rounded_full()
                        .bg(hsla(0.0, 0.0, 1.0, 0.95))
                        .border_1()
                        .border_color(hsla(0.0, 0.0, 0.0, 0.08)),
                ),
        )
}

// ── Sliders panel (composite) ────────────────────────────────────────────────

/// Renders a glass panel containing three example sliders: Brightness,
/// Volume, and Opacity.
///
/// Each slider is separated by a full-width thin separator line.
/// The panel adapts to dark / light mode via the frosted-glass panel base.
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::glass;
///
/// let panel = glass::sliders(true); // dark mode
/// ```
pub fn sliders(dark: bool) -> impl IntoElement {
    let track_inactive = if dark {
        hsla(0.0, 0.0, 1.0, 0.1)
    } else {
        hsla(0.0, 0.0, 0.0, 0.08)
    };

    panel(dark).child(
        div()
            .flex()
            .flex_col()
            .gap_4()
            .p_4()
            .child(slider_row(
                "Brightness",
                "☀️",
                0.7,
                BLUE,
                track_inactive,
                dark,
            ))
            .child(separator_full(dark))
            .child(slider_row(
                "Volume",
                "🔊",
                0.45,
                GREEN,
                track_inactive,
                dark,
            ))
            .child(separator_full(dark))
            .child(slider_row(
                "Opacity",
                "💧",
                0.85,
                MAUVE,
                track_inactive,
                dark,
            )),
    )
}
