//! Glass settings list, settings row, and iOS toggle components.
//!
//! A grouped list of settings rows inspired by the iOS Settings app.
//! Each row has a leading icon, a title, and either a trailing toggle
//! switch or a detail label with a chevron.

use gpui::{div, hsla, prelude::*, px};

use super::panel::{panel, separator};

// ── iOS Toggle ───────────────────────────────────────────────────────────────

/// Renders an iOS-style on/off toggle switch.
///
/// The toggle is purely visual (no state management). Pass `is_on` to
/// control whether it renders in the "on" (green) or "off" (gray) position.
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::glass;
///
/// let on  = glass::toggle(true, dark);
/// let off = glass::toggle(false, dark);
/// ```
pub fn toggle(is_on: bool, _dark: bool) -> impl IntoElement {
    let track_color = if is_on {
        hsla(0.38, 0.75, 0.50, 1.0) // green
    } else {
        hsla(0.0, 0.0, 0.5, 0.2)
    };
    let thumb_color = hsla(0.0, 0.0, 1.0, 0.95);

    div()
        .flex()
        .flex_row()
        .items_center()
        .w(px(51.0))
        .h(px(31.0))
        .rounded(px(16.0))
        .bg(track_color)
        .px(px(2.0))
        .when(is_on, |d| d.justify_end())
        .when(!is_on, |d| d.justify_start())
        .child(div().size(px(27.0)).rounded_full().bg(thumb_color))
}

// ── Settings Row ─────────────────────────────────────────────────────────────

/// Renders a single settings row with an icon, title, and trailing element.
///
/// If `has_toggle` is `true`, the row ends with an iOS toggle switch.
/// Otherwise it shows the optional `detail` text and a chevron ("›").
///
/// # Parameters
///
/// - `icon` — emoji or short text for the leading icon badge
/// - `title` — primary label
/// - `detail` — optional secondary text shown before the chevron
/// - `has_toggle` — whether to show a toggle instead of detail + chevron
/// - `text_primary` / `text_secondary` — HSLA colours for the text
/// - `dark` — dark-mode flag (forwarded to the toggle)
pub fn settings_row(
    icon: &str,
    title: &str,
    detail: Option<&str>,
    has_toggle: bool,
    text_primary: gpui::Hsla,
    text_secondary: gpui::Hsla,
    dark: bool,
) -> impl IntoElement {
    let row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap_3()
        .px_4()
        .py(px(11.0))
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(30.0))
                .rounded(px(7.0))
                .bg(hsla(0.6, 0.5, 0.5, 0.2))
                .text_base()
                .child(icon.to_string()),
        )
        .child(
            div()
                .flex_1()
                .text_base()
                .text_color(text_primary)
                .child(title.to_string()),
        );

    if has_toggle {
        row.child(toggle(true, dark))
    } else {
        let detail_text = detail.unwrap_or("");
        row.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_1()
                .child(
                    div()
                        .text_base()
                        .text_color(text_secondary)
                        .child(detail_text.to_string()),
                )
                .child(div().text_base().text_color(text_secondary).child("›")),
        )
    }
}

// ── Settings List (composite) ────────────────────────────────────────────────

/// Renders a complete iOS-style grouped settings list inside a glass panel.
///
/// The list includes five rows (Notifications, Dark Mode, Face ID, Wi-Fi,
/// Battery) separated by thin indented separators — matching the layout
/// found in the iOS Settings app.
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::glass;
///
/// let list = glass::settings_list(true); // dark mode
/// ```
pub fn settings_list(dark: bool) -> impl IntoElement {
    let text_primary = if dark {
        hsla(0.0, 0.0, 1.0, 0.92)
    } else {
        hsla(0.0, 0.0, 0.0, 0.85)
    };
    let text_secondary = if dark {
        hsla(0.0, 0.0, 1.0, 0.45)
    } else {
        hsla(0.0, 0.0, 0.0, 0.4)
    };

    panel(dark)
        .child(settings_row(
            "🔔",
            "Notifications",
            Some("On"),
            true,
            text_primary,
            text_secondary,
            dark,
        ))
        .child(separator(dark))
        .child(settings_row(
            "🌙",
            "Dark Mode",
            None,
            true,
            text_primary,
            text_secondary,
            dark,
        ))
        .child(separator(dark))
        .child(settings_row(
            "🔒",
            "Face ID",
            None,
            true,
            text_primary,
            text_secondary,
            dark,
        ))
        .child(separator(dark))
        .child(settings_row(
            "📶",
            "Wi-Fi",
            Some("Connected"),
            false,
            text_primary,
            text_secondary,
            dark,
        ))
        .child(separator(dark))
        .child(settings_row(
            "🔋",
            "Battery",
            Some("85%"),
            false,
            text_primary,
            text_secondary,
            dark,
        ))
}
