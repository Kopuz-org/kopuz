//! A newer Kopuz: offered across the top of the page, installed by the daemon where it can, linked where a package manager owns the install.

use api::{AvailableUpdate, UpdateStatus};
use components::format::{t, t_with};
use components::{Chip, ChipTone, icons, motion, style};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Context, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    Render, SharedString, StatefulInteractiveElement as _, Styled, Subscription, Window, div, px,
};
use store::UpdateModel;

const GLYPH: Pixels = px(12.);
const FILL: f32 = 0.45;

/// The update strip. It renders nothing while there is nothing to say.
pub struct UpdateBanner {
    // The version someone closed, so the strip stays gone until a newer one is out.
    dismissed: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl UpdateBanner {
    pub fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        let updates = UpdateModel::global(cx);
        Self {
            dismissed: None,
            _subscriptions: vec![cx.observe(&updates, |_, _, cx| cx.notify())],
        }
    }

    fn status(&self, cx: &App) -> Option<UpdateStatus> {
        let status = UpdateModel::global(cx).read(cx).status(cx).clone();
        let closed = |update: &AvailableUpdate| self.dismissed.as_ref() == Some(&update.version);
        match &status {
            UpdateStatus::Current => None,
            UpdateStatus::Available(update) | UpdateStatus::Failed(update) if closed(update) => {
                None
            }
            _ => Some(status),
        }
    }

    /// Whether the strip has anything to say, for whoever lays it out.
    pub fn is_showing(&self, cx: &App) -> bool {
        self.status(cx).is_some()
    }

    fn strip(cx: &App) -> gpui_kit::Div {
        h_flex()
            .w_full()
            .flex_shrink_0()
            .occlude()
            .items_center()
            .gap_2()
            .px_4()
            .py_1p5()
            .text_xs()
            .bg(style::black(FILL, cx))
            .border_b_1()
            .border_color(style::white(style::BORDER_SOFT, cx))
            .text_color(style::white(style::TEXT_SECONDARY, cx))
    }

    fn offer(
        &self,
        update: &AvailableUpdate,
        message: SharedString,
        action: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (over, pointer) = motion::hovered("update-dismiss-hover", window, cx);
        let dismiss = motion::amount(
            "update-dismiss-alpha",
            if over { 1. } else { 0.5 },
            motion::FAST,
            window,
            cx,
        );
        let url = update.release_url.clone();
        let version = update.version.clone();
        Self::strip(cx)
            .child(
                icons::icon(icons::DOWNLOAD)
                    .size(GLYPH)
                    .flex_shrink_0()
                    .text_color(cx.theme().primary),
            )
            .child(div().flex_1().min_w_0().truncate().child(message))
            .when(update.installable, |strip| {
                strip.child(
                    Chip::new("update-install")
                        .label(action)
                        .tone(ChipTone::Ink(cx.theme().primary))
                        .on_click(|_, _, cx| {
                            UpdateModel::global(cx).update(cx, |updates, cx| updates.install(cx));
                        }),
                )
            })
            .child(
                Chip::new("update-release")
                    .label(t("view_release"))
                    .on_click(move |_, _, cx| cx.open_url(&url)),
            )
            .child(
                div()
                    .id("update-dismiss")
                    .flex()
                    .flex_shrink_0()
                    .p_1()
                    .cursor_pointer()
                    .opacity(dismiss)
                    .on_hover(motion::track(pointer))
                    .tooltip(|window, cx| Tooltip::new(t("close")).build(window, cx))
                    .child(icons::icon(icons::CLOSE).size(GLYPH))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.dismissed = Some(version.clone());
                        cx.notify();
                    })),
            )
            .into_any_element()
    }
}

impl Render for UpdateBanner {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let version = |version: &str| [("version", version.to_string().into())];
        match self.status(cx) {
            None | Some(UpdateStatus::Current) => div().into_any_element(),
            Some(UpdateStatus::Available(update)) => {
                let message = t_with("update_banner_message", &version(&update.version));
                self.offer(&update, message, t("update_install"), window, cx)
            }
            Some(UpdateStatus::Failed(update)) => {
                let message = t_with("update_failed", &version(&update.version));
                self.offer(&update, message, t("update_retry"), window, cx)
            }
            Some(UpdateStatus::Downloading { version: v, .. }) => Self::strip(cx)
                .child(icons::icon(icons::DOWNLOAD).size(GLYPH).flex_shrink_0())
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(t_with("update_downloading", &version(&v))),
                )
                .into_any_element(),
            Some(UpdateStatus::Ready { .. }) => Self::strip(cx)
                .child(icons::icon(icons::REFRESH).size(GLYPH).flex_shrink_0())
                .child(div().flex_1().min_w_0().child(t("update_restarting")))
                .into_any_element(),
        }
    }
}
