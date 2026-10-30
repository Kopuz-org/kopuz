//! Settings → Interface → Playlists panel: whether the column is there, and how wide it opens.

use components::format::t;
use components::settings::{SettingRow, choice, nearest, section, toggle_row};
use gpui_kit::component::v_flex;
use gpui_kit::{
    App, AppContext as _, Context, IntoElement, ParentElement as _, Render, Styled, Window,
};
use store::LayoutModel;
use store::layout::{Panel, Section as Group};

/// The widths the panel offers, inside what the drag handle allows.
const WIDTHS: &[f32] = &[200., 260., 320., 380.];

pub fn register(cx: &mut App) {
    settings_interface::register(
        "playlists-panel",
        "interface_panel",
        2,
        Some(Group::Panel),
        |window, cx| cx.new(|cx| PanelSettings::new(window, cx)).into(),
        cx,
    );
}

struct PanelSettings;

impl PanelSettings {
    fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&LayoutModel::global(cx), |_, _, cx| cx.notify())
            .detach();
        Self
    }
}

fn pixels(width: f32) -> gpui_kit::SharedString {
    gpui_kit::SharedString::from(format!("{} px", width.round() as i32))
}

impl Render for PanelSettings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panel = store::layout(cx).panel.clone();
        let width = nearest(WIDTHS, panel.width).unwrap_or_else(|| Panel::default().width);
        v_flex()
            .w_full()
            .child(
                section(t("interface_panel"))
                    .children(
                        toggle_row(
                            "interface-panel-shown",
                            t("interface_panel_shown"),
                            panel.shown,
                            |on, cx| LayoutModel::edit(cx, |layout| layout.panel.shown = on),
                        )
                        .description(t("interface_panel_shown_description"))
                        .shown(cx),
                    )
                    .children(
                        SettingRow::new(t("interface_panel_width"))
                            .description(t("interface_panel_width_description"))
                            .control(choice(
                                "interface-panel-width",
                                &width,
                                WIDTHS.iter().map(|step| (*step, pixels(*step))).collect(),
                                |value, cx| {
                                    LayoutModel::edit(cx, |layout| layout.panel.width = value)
                                },
                            ))
                            .shown(cx),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_width_reads_as_pixels() {
        assert_eq!(pixels(260.), "260 px");
    }
}
