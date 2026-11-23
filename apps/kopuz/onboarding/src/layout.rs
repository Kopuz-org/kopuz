use components::format::t;
use components::settings::{SettingRow, choice, toggle_row};
use components::skin::{self, Nav};
use gpui_kit::{App, Div, ParentElement as _};
use store::LayoutModel;
use store::layout::{NavPosition, Side};

fn sides() -> Vec<(Side, gpui_kit::SharedString)> {
    vec![
        (Side::Left, t("interface_sheet_side_left")),
        (Side::Right, t("interface_sheet_side_right")),
    ]
}

pub(crate) fn card(cx: &App) -> Div {
    let layout = store::layout(cx);
    let strip = skin::arrangement().nav == Nav::Strip;
    let position = layout.nav.position;
    let nav = strip.then(|| {
        SettingRow::new(t("interface_nav_position"))
            .description(t("interface_nav_position_description"))
            .control(choice(
                "onboarding-nav-position",
                &position,
                vec![
                    (NavPosition::Top, t("interface_nav_position_top")),
                    (NavPosition::Left, t("interface_nav_position_left")),
                ],
                |value, cx| LayoutModel::edit(cx, |layout| layout.nav.position = value),
            ))
    });
    let controls = (strip && position == NavPosition::Left).then(|| {
        toggle_row(
            "onboarding-nav-controls-up-top",
            t("interface_nav_controls_up_top"),
            layout.nav.controls_up_top,
            |on, cx| LayoutModel::edit(cx, |layout| layout.nav.controls_up_top = on),
        )
        .description(t("interface_nav_controls_up_top_description"))
    });
    let panel = SettingRow::new(t("interface_panel_side"))
        .description(t("interface_panel_side_description"))
        .control(choice(
            "onboarding-panel-side",
            &layout.panel.side,
            sides(),
            |value, cx| LayoutModel::edit(cx, |layout| layout.panel.side = value),
        ));
    let queue = SettingRow::new(t("interface_queue_side"))
        .description(t("interface_queue_side_description"))
        .control(choice(
            "onboarding-queue-side",
            &layout.interface.queue_side,
            sides(),
            |value, cx| LayoutModel::edit(cx, |layout| layout.interface.queue_side = value),
        ));
    crate::ui::card(cx)
        .children(nav)
        .children(controls)
        .child(panel)
        .child(queue)
}
