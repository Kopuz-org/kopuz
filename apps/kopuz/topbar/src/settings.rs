//! Settings → Interface → Navigation: where the bar sits, what it holds, and where the app opens.

use components::format::t;
use components::settings::{SettingRow, choice, order_row, section};
use gpui_kit::component::v_flex;
use gpui_kit::{
    App, AppContext as _, Context, IntoElement, ParentElement as _, Render, Styled, Window,
};
use store::LayoutModel;
use store::layout::{DESTINATIONS, NavPosition, Section as Group};

pub fn register(cx: &mut App) {
    settings_interface::register(
        "navigation",
        "interface_nav",
        1,
        Some(Group::Nav),
        |window, cx| cx.new(|cx| Navigation::new(window, cx)).into(),
        cx,
    );
}

struct Navigation;

impl Navigation {
    fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&LayoutModel::global(cx), |_, _, cx| cx.notify())
            .detach();
        Self
    }
}

impl Render for Navigation {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let nav = &store::layout(cx).nav;
        let position = nav.position;
        let start = nav.start.clone();
        let rows = DESTINATIONS
            .iter()
            .filter_map(|name| {
                order_row(
                    "nav",
                    name,
                    t(crate::tabs::label(name)),
                    nav,
                    |layout| &mut layout.nav,
                    cx,
                )
                .shown(cx)
            })
            .collect::<Vec<_>>();
        v_flex()
            .w_full()
            .child(
                section(t("interface_nav"))
                    .children(
                        SettingRow::new(t("interface_nav_position"))
                            .description(t("interface_nav_position_description"))
                            .control(choice(
                                "interface-nav-position",
                                &position,
                                vec![
                                    (NavPosition::Top, t("interface_nav_position_top")),
                                    (NavPosition::Left, t("interface_nav_position_left")),
                                ],
                                |value, cx| {
                                    LayoutModel::edit(cx, |layout| layout.nav.position = value)
                                },
                            ))
                            .shown(cx),
                    )
                    .children(
                        SettingRow::new(t("interface_nav_start"))
                            .description(t("interface_nav_start_description"))
                            .control(choice(
                                "interface-nav-start",
                                &start,
                                DESTINATIONS
                                    .iter()
                                    .map(|name| (name.to_string(), t(crate::tabs::label(name))))
                                    .collect(),
                                |value, cx| {
                                    LayoutModel::edit(cx, |layout| layout.nav.start = value)
                                },
                            ))
                            .shown(cx),
                    )
                    .children(rows),
            )
            .into_any_element()
    }
}
