//! The link could not be set up at all: a window that says why, rather than no window at all.

use components::format::t;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::{
    Context, IntoElement, ParentElement as _, Pixels, Render, SharedString, Styled, Window, div, px,
};

/// The reason reads as a paragraph, not as one long line across the window.
const REASON_WIDTH: Pixels = px(420.);

/// Shown instead of quitting when `link::init` refuses the address it was given.
pub struct StartupError {
    reason: SharedString,
}

impl StartupError {
    pub fn new(reason: impl Into<SharedString>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl Render for StartupError {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .p_8()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(div().text_lg().child(t("daemon_unreachable")))
            .child(
                div()
                    .max_w(REASON_WIDTH)
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.reason.clone()),
            )
            .child(
                Button::new("quit")
                    .primary()
                    .label(t("quit"))
                    .on_click(|_, _, cx| cx.quit()),
            )
    }
}
