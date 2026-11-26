//! The frames the Plugins panel draws with: a card, a name and the lines under it.

use components::style;
use gpui_kit::component::v_flex;
use gpui_kit::{App, Div, FontWeight, ParentElement as _, SharedString, Styled, div};

pub use components::settings::section;

/// `text-white/60`, a card's secondary lines.
const HINT: f32 = 0.60;

pub fn card(_: &App) -> Div {
    v_flex().w_full().gap_3().px_5().py_4()
}

/// `text-sm font-medium text-white`: the plugin's name.
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
