//! Settings → Interface → Playlists panel: whether the column is there, and how wide it opens.

use components::format::t;
use components::settings::{SettingRow, choice, nearest, section, toggle_row};
use components::{icons, style};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, IntoElement, ParentElement as _, Render, SharedString, Styled,
    Window,
};
use store::LayoutModel;
use store::layout::{Panel, PinKind, Section as Group, Side};

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

fn pin_rows(panel: &Panel, cx: &App) -> Vec<SettingRow> {
    let count = panel.pins.len();
    if count == 0 {
        return SettingRow::new(t("pinned"))
            .description(t("interface_panel_no_pins"))
            .shown(cx)
            .into_iter()
            .collect();
    }
    panel
        .pins
        .iter()
        .enumerate()
        .filter_map(|(index, pin)| {
            let kind = match pin.kind {
                PinKind::Album => t("album"),
                PinKind::Artist => t("artist"),
                PinKind::Playlist => t("pinned_playlist"),
                PinKind::Folder => t("pinned_folder"),
            };
            let button = |name: &str,
                          glyph: &'static str,
                          tooltip: SharedString,
                          enabled: bool,
                          to: Option<usize>| {
                let (kind, id) = (pin.kind, pin.id.clone());
                Button::new(SharedString::from(format!(
                    "pin-{name}-{:?}-{}",
                    pin.kind, pin.id
                )))
                .small()
                .custom(style::quiet_button(style::slate_400(cx), cx))
                .icon(icons::icon(glyph))
                .tooltip(tooltip)
                .disabled(!enabled)
                .on_click(move |_, _, cx| {
                    let id = id.clone();
                    LayoutModel::edit(cx, move |layout| match to {
                        Some(to) => layout.panel.move_pin(kind, &id, to),
                        None => layout
                            .panel
                            .pins
                            .retain(|held| !(held.kind == kind && held.id == id)),
                    });
                })
            };
            SettingRow::new(SharedString::from(pin.name.clone()))
                .description(kind)
                .control(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .child(button(
                            "up",
                            icons::CHEVRON_UP,
                            t("move_up"),
                            index > 0,
                            Some(index.saturating_sub(1)),
                        ))
                        .child(button(
                            "down",
                            icons::CHEVRON_DOWN,
                            t("move_down"),
                            index + 1 < count,
                            Some(index + 2),
                        ))
                        .child(button(
                            "remove",
                            icons::CLOSE,
                            t("unpin_from_sidebar"),
                            true,
                            None,
                        )),
                )
                .shown(cx)
        })
        .collect()
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
                        SettingRow::new(t("interface_panel_side"))
                            .description(t("interface_panel_side_description"))
                            .control(choice(
                                "interface-panel-side",
                                &panel.side,
                                vec![
                                    (Side::Left, t("interface_sheet_side_left")),
                                    (Side::Right, t("interface_sheet_side_right")),
                                ],
                                |value, cx| {
                                    LayoutModel::edit(cx, |layout| layout.panel.side = value)
                                },
                            ))
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
                    )
                    .children(pin_rows(&panel, cx)),
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
