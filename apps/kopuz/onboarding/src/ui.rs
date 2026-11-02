//! The frames the guide's steps share.

use components::SettingRow;
use components::format::t;
use components::settings::Toggle;
use components::style;
use config::SettingsEdit;
use gpui_kit::component::v_flex;
use gpui_kit::{App, Div, ParentElement as _, SharedString, Styled, div};
use store::ConfigModel;

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

/// One switch bound to its config key, with a line saying what turning it on does.
pub(crate) fn toggle(
    key: &'static str,
    title: &'static str,
    description: &'static str,
    on: bool,
    edit: fn(bool) -> SettingsEdit,
    cx: &App,
) -> SettingRow {
    SettingRow::new(t(title))
        .description(t(description))
        .setting(
            Toggle::new(SharedString::from(format!("onboarding-{key}")))
                .enabled(on)
                .on_change(move |on, _, cx| ConfigModel::edit(cx, edit(*on))),
        )
        .bind(key, cx)
}
