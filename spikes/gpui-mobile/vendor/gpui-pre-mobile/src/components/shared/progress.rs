//! Shared progress bar components — labelled progress bar tracks.
//!
//! Progress bars visualise the completion status of a task or metric.
//! Each progress row displays a label, a percentage value, and a
//! coloured track with a fill proportional to the value.
//!
//! Two public functions are provided:
//!
//! - [`progress_row`] — a single labelled progress bar with configurable
//!   value, colour, and track colour
//! - [`progress_bars`] — a composite card containing Storage, Memory, CPU,
//!   and Battery progress indicators

use gpui::{div, prelude::*, px, relative, rgb};

// ── Colour constants (Catppuccin Mocha) ──────────────────────────────────────

const SURFACE0: u32 = 0x313244;
const SURFACE1: u32 = 0x45475a;
const TEXT: u32 = 0xcdd6f4;
const BLUE: u32 = 0x89b4fa;
const GREEN: u32 = 0xa6e3a1;
const RED: u32 = 0xf38ba8;
const YELLOW: u32 = 0xf9e2af;

// ── Single progress row ──────────────────────────────────────────────────────

/// Renders a single labelled progress bar.
///
/// The layout:
/// ```text
/// Storage                            72%
/// ┌──────────────────────────────────────┐
/// │ ████████████████████████░░░░░░░░░░░░ │
/// └──────────────────────────────────────┘
/// ```
///
/// # Parameters
///
/// - `label` — descriptive text (e.g. "Storage", "CPU")
/// - `progress` — fill ratio in the range `0.0..=1.0`
/// - `color` — RGB colour for the active fill
/// - `track` — RGB colour for the inactive portion of the track
/// - `text_color` — RGB colour for the label text
/// - `sub_text` — RGB colour for the percentage text
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::shared;
///
/// let bar = shared::progress_row(
///     "Storage", 0.72, 0x89b4fa, 0x45475a, 0xcdd6f4, 0xa6adc8,
/// );
/// ```
pub fn progress_row(
    label: &str,
    progress: f32,
    color: u32,
    track: u32,
    text_color: u32,
    sub_text: u32,
) -> impl IntoElement {
    let pct = (progress * 100.0) as u32;
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .flex_row()
                .justify_between()
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(text_color))
                        .child(label.to_string()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(sub_text))
                        .child(format!("{}%", pct)),
                ),
        )
        .child(
            div()
                .w_full()
                .h(px(6.0))
                .rounded(px(3.0))
                .bg(rgb(track))
                .child(
                    div()
                        .h_full()
                        .rounded(px(3.0))
                        .bg(rgb(color))
                        .w(relative(progress)),
                ),
        )
}

// ── Progress bars card (composite) ───────────────────────────────────────────

/// Renders a card containing four example progress bars: Storage, Memory,
/// CPU, and Battery.
///
/// The card has a rounded background and adapts to dark / light mode.
/// Each progress bar uses a different accent colour from the Catppuccin
/// Mocha palette.
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::shared;
///
/// let card = shared::progress_bars(true); // dark mode
/// ```
pub fn progress_bars(dark: bool) -> impl IntoElement {
    let card_bg = if dark { SURFACE0 } else { 0xe6e9ef };
    let text_color = if dark { TEXT } else { 0x4c4f69 };
    let sub_text: u32 = if dark { 0xa6adc8 } else { 0x6c6f85 };
    let track_color = if dark { SURFACE1 } else { 0xdce0e8 };

    div()
        .flex()
        .flex_col()
        .gap_2()
        .p_4()
        .rounded(px(12.0))
        .bg(rgb(card_bg))
        .child(progress_row(
            "Storage",
            0.72,
            BLUE,
            track_color,
            text_color,
            sub_text,
        ))
        .child(progress_row(
            "Memory",
            0.45,
            GREEN,
            track_color,
            text_color,
            sub_text,
        ))
        .child(progress_row(
            "CPU",
            0.90,
            RED,
            track_color,
            text_color,
            sub_text,
        ))
        .child(progress_row(
            "Battery",
            0.58,
            YELLOW,
            track_color,
            text_color,
            sub_text,
        ))
}
