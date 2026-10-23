//! The frames the Library and Connectivity panels share: cards, a name and its hint, the host-access warning.

use components::{Callout, CalloutTone, icons, style};
use gpui_kit::component::v_flex;
use gpui_kit::{App, Div, FontWeight, ParentElement as _, SharedString, Styled, div};

pub use components::settings::section;

// Flathub refuses `org.freedesktop.Flatpak` in the manifest, so the user grants it by hand.
const HOST_ACCESS_COMMAND: &str =
    "flatpak override --user --talk-name=org.freedesktop.Flatpak moe.kopuz.kopuz";

/// `text-white/60`, a card's secondary lines.
const HINT: f32 = 0.60;

/// The inset a settings card gives its contents, the same one a stacked row has.
pub fn card(_: &App) -> Div {
    v_flex().w_full().gap_3().px_5().py_4()
}

/// `text-sm font-medium text-white`: the name of whatever the card is about.
pub fn label(text: SharedString) -> Div {
    div()
        .min_w_0()
        .truncate()
        .text_sm()
        .font_weight(FontWeight::MEDIUM)
        .child(text)
}

/// `text-xs text-white/60`: a line under that name.
pub fn hint(text: SharedString, cx: &App) -> Div {
    div()
        .text_xs()
        .text_color(style::white(HINT, cx))
        .child(text)
}

/// Said when the daemon cannot reach a browser on the host; the command that fixes it is copyable.
pub fn host_access_warning(id: impl Into<SharedString>, message: SharedString) -> Callout {
    Callout::new(message)
        .id(id)
        .tone(CalloutTone::Warning)
        .icon(icons::WARNING)
        .copyable(HOST_ACCESS_COMMAND)
}
