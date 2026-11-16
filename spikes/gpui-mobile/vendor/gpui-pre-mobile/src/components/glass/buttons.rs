//! Glass button components — tinted, plain, and capsule styles.
//!
//! These buttons follow the Apple Glass design language with translucent
//! backgrounds, subtle tinting, and rounded corners. Three styles are
//! provided:
//!
//! - **Tinted** — coloured translucent background with matching text
//! - **Plain** — neutral gray translucent background
//! - **Capsule** — pill-shaped filled or outlined buttons (SF style)

use gpui::{div, hsla, prelude::*, px, rgb};

// ── Constants ────────────────────────────────────────────────────────────────

/// Default blue accent used for capsule buttons (Catppuccin Mocha blue).
const BLUE: u32 = 0x89b4fa;

// ── Tinted button ────────────────────────────────────────────────────────────

/// A tinted glass button with a translucent coloured background.
///
/// The `hue`, `sat`, and `light` parameters control the HSL base colour.
/// The background uses 18 % opacity of that colour, and the text is rendered
/// slightly brighter for contrast.
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::glass;
///
/// let btn = glass::button_tinted("Primary", 0.6, 0.6, 0.5);
/// ```
pub fn button_tinted(label: &str, hue: f32, sat: f32, light: f32) -> impl IntoElement {
    div()
        .px_4()
        .py(px(8.0))
        .rounded(px(10.0))
        .bg(hsla(hue, sat, light, 0.18))
        .text_sm()
        .text_color(hsla(hue, sat, light + 0.15, 1.0))
        .child(label.to_string())
}

// ── Plain button ─────────────────────────────────────────────────────────────

/// A plain glass button with a neutral translucent background.
///
/// Adapts to dark / light mode with appropriate contrast.
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::glass;
///
/// let btn = glass::button_plain("Cancel", dark);
/// ```
pub fn button_plain(label: &str, dark: bool) -> impl IntoElement {
    let bg = if dark {
        hsla(0.0, 0.0, 1.0, 0.08)
    } else {
        hsla(0.0, 0.0, 0.0, 0.05)
    };
    let fg = if dark {
        hsla(0.0, 0.0, 1.0, 0.7)
    } else {
        hsla(0.0, 0.0, 0.0, 0.55)
    };

    div()
        .px_4()
        .py(px(8.0))
        .rounded(px(10.0))
        .bg(bg)
        .text_sm()
        .text_color(fg)
        .child(label.to_string())
}

// ── Buttons row (composite) ──────────────────────────────────────────────────

/// Renders a complete buttons showcase with three rows:
///
/// 1. **Tinted** — Primary, Success, Danger, Warning
/// 2. **Plain** — Cancel, Skip, Later
/// 3. **Capsule** — filled "Get Started" + outlined "Learn More"
///
/// This is a convenience composite; you can also use the individual button
/// functions ([`button_tinted`], [`button_plain`]) directly.
pub fn buttons_row(dark: bool) -> impl IntoElement {
    let text_white = hsla(0.0, 0.0, 1.0, 0.92);

    div()
        .flex()
        .flex_col()
        .gap_3()
        // Row 1: Tinted glass buttons (iOS style)
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(button_tinted("Primary", 0.6, 0.6, 0.5))
                .child(button_tinted("Success", 0.38, 0.7, 0.45))
                .child(button_tinted("Danger", 0.0, 0.7, 0.5))
                .child(button_tinted("Warning", 0.1, 0.8, 0.5)),
        )
        // Row 2: Plain glass (gray) buttons
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(button_plain("Cancel", dark))
                .child(button_plain("Skip", dark))
                .child(button_plain("Later", dark)),
        )
        // Row 3: Pill / capsule buttons (SF style)
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(
                    div()
                        .px_5()
                        .py(px(10.0))
                        .rounded(px(20.0))
                        .bg(rgb(BLUE))
                        .text_sm()
                        .text_color(text_white)
                        .child("Get Started"),
                )
                .child(
                    div()
                        .px_5()
                        .py(px(10.0))
                        .rounded(px(20.0))
                        .border_1()
                        .border_color(rgb(BLUE))
                        .text_sm()
                        .text_color(rgb(BLUE))
                        .child("Learn More"),
                ),
        )
}
