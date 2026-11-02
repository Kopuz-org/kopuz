//! The one card of tips shown after setup, until it is closed.

use components::format::t;
use components::{Keycap, icons, motion, style};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    Styled, Window, div, px,
};
use store::{LayoutModel, SetupModel};

const WIDTH: Pixels = px(320.);
const GLYPH: Pixels = px(14.);
const RISE: f32 = 6.;

/// Only once setup is done and the guide is down, and never again once closed.
pub fn wanted(complete: bool, guide_up: bool, dismissed: bool) -> bool {
    complete && !guide_up && !dismissed
}

fn tip(glyph: &'static str, lead: Option<AnyElement>, key: &'static str, cx: &App) -> AnyElement {
    h_flex()
        .gap_2()
        .items_start()
        .child(
            icons::icon(glyph)
                .size(GLYPH)
                .flex_shrink_0()
                .text_color(style::slate_400(cx)),
        )
        .children(lead)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .text_color(style::white(style::TEXT_SECONDARY, cx))
                .child(t(key)),
        )
        .into_any_element()
}

pub(crate) fn card(window: &mut Window, cx: &mut App) -> Option<AnyElement> {
    let setup = SetupModel::global(cx).read(cx);
    let shown = wanted(
        setup.is_complete(),
        setup.showing(),
        store::layout(cx).first_use_hint_dismissed,
    );
    if !shown {
        return None;
    }
    let fade = motion::enter("first-use-hint", motion::MEDIUM, window, cx);
    Some(
        v_flex()
            .absolute()
            .bottom_4()
            .right_4()
            .w(WIDTH)
            .child(
                v_flex()
                    .relative()
                    .top(px(RISE * (1. - fade)))
                    .opacity(fade)
                    .occlude()
                    .w_full()
                    .gap_3()
                    .p_4()
                    .rounded(style::radius_xl())
                    .border_1()
                    .border_color(style::white(style::BORDER, cx))
                    .bg(cx.theme().popover)
                    .shadow_lg()
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(t("tips_title")),
                            )
                            .child(
                                Button::new("first-use-hint-close")
                                    .ghost()
                                    .xsmall()
                                    .icon(icons::icon(icons::CLOSE))
                                    .tooltip(t("close"))
                                    .on_click(|_, _, cx| {
                                        LayoutModel::edit(cx, |layout| {
                                            layout.first_use_hint_dismissed = true
                                        })
                                    }),
                            ),
                    )
                    .child(tip(
                        icons::SEARCH,
                        Some(Keycap::new("ctrl-k").into_any_element()),
                        "tips_search",
                        cx,
                    ))
                    .child(tip(icons::MUSIC, None, "tips_now_playing", cx))
                    .child(tip(icons::LIBRARY, None, "tips_source_switcher", cx)),
            )
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tips_wait_for_setup_and_stay_closed() {
        assert!(wanted(true, false, false));
        assert!(!wanted(false, false, false));
        assert!(!wanted(true, true, false));
        assert!(!wanted(true, false, true));
    }
}
