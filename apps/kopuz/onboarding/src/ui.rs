//! The frames the guide's steps share.

use components::style;
use gpui_kit::component::v_flex;
use gpui_kit::{App, Div, ParentElement as _, SharedString, Styled, div};

pub(crate) fn hint(text: SharedString, cx: &App) -> Div {
    div().text_sm().text_color(style::slate_400(cx)).child(text)
}

/// A raised panel holding a few rows, like a settings section.
pub(crate) fn card(cx: &App) -> Div {
    v_flex()
        .w_full()
        .overflow_hidden()
        .rounded(style::radius_xl())
        .border_1()
        .border_color(style::white(style::BORDER, cx))
        .bg(style::white(style::RAISED, cx))
}
