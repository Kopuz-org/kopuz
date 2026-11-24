//! The knobs that belong to no single package: motion, accent and how big everything is drawn.

use components::format::{t, t_with};
use components::settings::{SettingRow, choice, nearest, section, select_width, toggle_row};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::v_flex;
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, SharedString,
    Styled, Subscription, Window,
};
use store::LayoutModel;
use store::layout::{Accent, Ambient, Motion, PageWidth, Section as Group};

/// The scale steps the size setting offers, inside `layout::SCALE_RANGE`.
const STEPS: &[f32] = &[0.8, 0.9, 1.0, 1.1, 1.25, 1.4];

pub fn register(cx: &mut App) {
    crate::register(
        "general",
        "interface_general",
        0,
        Some(Group::Interface),
        |window, cx| cx.new(|cx| General::new(window, cx)).into(),
        cx,
    );
}

struct General {
    font: Entity<InputState>,
    /// The typed family when the system has no such font, which draws in the theme's instead.
    missing: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl General {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let held = store::layout(cx).interface.font.clone().unwrap_or_default();
        let font = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t("interface_font_placeholder"))
                .default_value(held)
        });
        let missing = unknown_font(store::layout(cx).interface.font.as_deref(), cx);
        let subscription = cx.subscribe(&font, |this: &mut Self, state, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                let typed = state.read(cx).value().trim().to_string();
                let family = (!typed.is_empty()).then_some(typed);
                this.missing = unknown_font(family.as_deref(), cx);
                LayoutModel::edit(cx, |layout| layout.interface.font = family);
            }
        });
        let layout_subscription = cx.observe_in(
            &LayoutModel::global(cx),
            window,
            |this: &mut Self, _, window, cx| {
                let family = store::layout(cx).interface.font.clone().unwrap_or_default();
                if this.font.read(cx).value().as_str() != family {
                    this.font
                        .update(cx, |font, cx| font.set_value(family, window, cx));
                }
                cx.notify();
            },
        );
        Self {
            font,
            missing,
            _subscriptions: vec![subscription, layout_subscription],
        }
    }
}

impl Render for General {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let layout = store::layout(cx);
        let motion = layout.motion;
        let accent = layout.accent;
        let ambient = layout.ambient;
        let scale = layout.interface.scale;
        let page_width = layout.interface.page_width;
        let similar_artists = layout.interface.similar_artists;
        v_flex()
            .w_full()
            .child(
                section(t("interface_general"))
                    .children(
                        SettingRow::new(t("interface_motion"))
                            .description(t("interface_motion_description"))
                            .control(choice(
                                "interface-motion",
                                &motion,
                                vec![
                                    (Motion::Full, t("interface_motion_full")),
                                    (Motion::Reduced, t("interface_motion_reduced")),
                                    (Motion::Off, t("interface_motion_off")),
                                ],
                                |value, cx| LayoutModel::edit(cx, |layout| layout.motion = value),
                            ))
                            .shown(cx),
                    )
                    .children(
                        SettingRow::new(t("interface_accent"))
                            .description(t("interface_accent_description"))
                            .control(choice(
                                "interface-accent",
                                &accent,
                                vec![
                                    (Accent::On, t("interface_accent_on")),
                                    (Accent::SheetOnly, t("interface_accent_sheet")),
                                    (Accent::Off, t("interface_accent_off")),
                                ],
                                |value, cx| LayoutModel::edit(cx, |layout| layout.accent = value),
                            ))
                            .shown(cx),
                    )
                    // The light is the accent carried furthest, so it is only offered where it reaches.
                    .children(
                        (accent == Accent::On)
                            .then(|| {
                                SettingRow::new(t("interface_ambient"))
                                    .description(t("interface_ambient_description"))
                                    .control(choice(
                                        "interface-ambient",
                                        &ambient,
                                        vec![
                                            (Ambient::Glow, t("interface_ambient_glow")),
                                            (Ambient::Full, t("interface_ambient_full")),
                                            (Ambient::Off, t("interface_ambient_off")),
                                        ],
                                        |value, cx| {
                                            LayoutModel::edit(cx, |layout| layout.ambient = value)
                                        },
                                    ))
                                    .shown(cx)
                            })
                            .flatten(),
                    )
                    .children(
                        SettingRow::new(t("interface_scale"))
                            .description(t("interface_scale_description"))
                            .control(choice(
                                "interface-scale",
                                &nearest(STEPS, scale).unwrap_or(1.),
                                STEPS.iter().map(|step| (*step, percent(*step))).collect(),
                                |value, cx| {
                                    LayoutModel::edit(cx, |layout| layout.interface.scale = value)
                                },
                            ))
                            .shown(cx),
                    )
                    .children(
                        SettingRow::new(t("interface_page_width"))
                            .description(t("interface_page_width_description"))
                            .control(choice(
                                "interface-page-width",
                                &page_width,
                                vec![
                                    (PageWidth::Standard, t("interface_page_width_standard")),
                                    (PageWidth::Wide, t("interface_page_width_wide")),
                                    (PageWidth::Full, t("interface_page_width_full")),
                                ],
                                |value, cx| {
                                    LayoutModel::edit(cx, |layout| {
                                        layout.interface.page_width = value
                                    })
                                },
                            ))
                            .shown(cx),
                    )
                    .children(
                        toggle_row(
                            "interface-similar-artists",
                            t("interface_similar_artists"),
                            similar_artists,
                            |on, cx| {
                                LayoutModel::edit(cx, |layout| {
                                    layout.interface.similar_artists = on
                                })
                            },
                        )
                        .description(t("interface_similar_artists_description"))
                        .shown(cx),
                    )
                    .children(
                        SettingRow::new(t("interface_font"))
                            .description(match self.missing.clone() {
                                Some(font) => t_with(
                                    "interface_font_missing",
                                    &[("font", font.to_string().into())],
                                ),
                                None => t("interface_font_description"),
                            })
                            .control(Input::new(&self.font).w(select_width()))
                            .shown(cx),
                    ),
            )
            .into_any_element()
    }
}

/// `family`, when it names no font the system has.
fn unknown_font(family: Option<&str>, cx: &gpui_kit::App) -> Option<SharedString> {
    let family = family?;
    let known = cx
        .text_system()
        .all_font_names()
        .iter()
        .any(|name| name.eq_ignore_ascii_case(family));
    (!known).then(|| SharedString::from(family.to_string()))
}

fn percent(scale: f32) -> SharedString {
    SharedString::from(format!("{}%", (scale * 100.).round() as i32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_reads_as_a_percentage() {
        assert_eq!(percent(1.25), "125%");
        assert_eq!(percent(0.8), "80%");
    }
}
