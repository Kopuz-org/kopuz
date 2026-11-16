//! Glass notification banner components.
//!
//! Banner-style notification cards inspired by iOS notification centre.
//! Each notification has a leading app icon, an app name, a message body,
//! and a timestamp — all rendered inside a frosted-glass panel.

use gpui::{div, hsla, prelude::*, px, rgb};

use super::panel::panel;

// ── Constants ────────────────────────────────────────────────────────────────

/// Default blue accent for notification icon backgrounds (Catppuccin Mocha blue).
const BLUE: u32 = 0x89b4fa;

// ── Single notification ──────────────────────────────────────────────────────

/// Renders a single notification banner inside a glass panel.
///
/// The banner layout:
/// ```text
/// ┌─────────────────────────────────────┐
/// │  [icon]  App Name          2m ago   │
/// │          Message body text…         │
/// └─────────────────────────────────────┘
/// ```
///
/// # Parameters
///
/// - `icon` — emoji or short text for the app icon badge
/// - `app` — application name shown in bold
/// - `message` — notification body text
/// - `time` — timestamp string (e.g. "now", "2m ago")
/// - `text_primary` / `text_secondary` — HSLA colours for the text
/// - `dark` — dark-mode flag (forwarded to the glass panel)
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::glass;
///
/// let banner = glass::notification(
///     "📱", "Messages", "Hey!", "now",
///     text_primary, text_secondary, true,
/// );
/// ```
pub fn notification(
    icon: &str,
    app: &str,
    message: &str,
    time: &str,
    text_primary: gpui::Hsla,
    text_secondary: gpui::Hsla,
    dark: bool,
) -> impl IntoElement {
    panel(dark).child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_3()
            .p_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(38.0))
                    .rounded(px(9.0))
                    .bg(rgb(BLUE))
                    .text_lg()
                    .child(icon.to_string()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .gap(px(2.0))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_between()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(text_primary)
                                    .child(app.to_string()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(text_secondary)
                                    .child(time.to_string()),
                            ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(text_secondary)
                            .child(message.to_string()),
                    ),
            ),
    )
}

// ── Notification banners (composite) ─────────────────────────────────────────

/// Renders a pair of example notification banners stacked vertically.
///
/// This is a convenience composite that demonstrates how [`notification`]
/// looks in practice — a Messages banner and a Mail banner with appropriate
/// timestamps.
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::glass;
///
/// let banners = glass::notification_banners(true); // dark mode
/// ```
pub fn notification_banners(dark: bool) -> impl IntoElement {
    let text_primary = if dark {
        hsla(0.0, 0.0, 1.0, 0.92)
    } else {
        hsla(0.0, 0.0, 0.0, 0.85)
    };
    let text_secondary = if dark {
        hsla(0.0, 0.0, 1.0, 0.55)
    } else {
        hsla(0.0, 0.0, 0.0, 0.45)
    };

    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(notification(
            "📱",
            "Messages",
            "Hey! Are you coming to the meetup?",
            "now",
            text_primary,
            text_secondary,
            dark,
        ))
        .child(notification(
            "📧",
            "Mail",
            "Your order has been shipped",
            "2m ago",
            text_primary,
            text_secondary,
            dark,
        ))
}
