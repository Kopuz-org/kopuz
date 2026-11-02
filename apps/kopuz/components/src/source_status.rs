//! A source's link status as a pulsing dot, or as a labelled line with its sign-in.

use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Animation, AnimationExt as _, App, ClickEvent, Hsla, IntoElement, ParentElement as _, Pixels,
    RenderOnce, SharedString, Styled, Window, div, pulsating_between, px,
};
use store::{SignInWait, SourceStatus};

use crate::format::t;
use crate::style;

const DOT: Pixels = px(6.);

fn ink(status: SourceStatus, cx: &App) -> Hsla {
    match status {
        SourceStatus::Connected => cx.theme().success,
        SourceStatus::Connecting => cx.theme().warning,
        SourceStatus::Disconnected => cx.theme().danger,
    }
}

/// The small coloured dot for a status, pulsing while the daemon is still checking.
#[derive(IntoElement)]
pub struct StatusDot {
    status: SourceStatus,
}

impl StatusDot {
    pub fn new(status: SourceStatus) -> Self {
        Self { status }
    }
}

impl RenderOnce for StatusDot {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let dot = div()
            .w(DOT)
            .h(DOT)
            .flex_shrink_0()
            .rounded_full()
            .bg(ink(self.status, cx));
        if self.status != SourceStatus::Connecting {
            return dot.into_any_element();
        }
        dot.with_animation(
            "source-status",
            Animation::new(Duration::from_millis(1100))
                .repeat()
                .with_easing(pulsating_between(0.4, 1.)),
            |dot, delta| dot.opacity(delta),
        )
        .into_any_element()
    }
}

type SignIn = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// A status in words, and while it is down either a spinner for the browser sign-in or the button to start one.
#[derive(IntoElement)]
pub struct StatusLine {
    status: SourceStatus,
    waiting: Option<SignInWait>,
    label: Option<SharedString>,
    on_sign_in: Option<SignIn>,
}

impl StatusLine {
    pub fn new(status: SourceStatus) -> Self {
        Self {
            status,
            waiting: None,
            label: None,
            on_sign_in: None,
        }
    }

    /// A sign-in is under way: it shows a spinner saying on what, and offers no second one.
    pub fn waiting(mut self, waiting: Option<SignInWait>) -> Self {
        self.waiting = waiting;
        self
    }

    /// Say this instead of the status's own word, in the quiet ink of a hint.
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The sign-in button shown while the source is disconnected and none is under way.
    pub fn on_sign_in(
        mut self,
        on_sign_in: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_sign_in = Some(Rc::new(on_sign_in));
        self
    }
}

impl RenderOnce for StatusLine {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let (label, color) = match self.label {
            Some(label) => (label, style::white(style::TEXT_SECONDARY, cx)),
            None => (
                t(match self.status {
                    SourceStatus::Connected => "connected",
                    SourceStatus::Connecting => "connecting",
                    SourceStatus::Disconnected => "disconnected",
                }),
                ink(self.status, cx),
            ),
        };
        let waiting = self.waiting.is_some();
        let down = self.status == SourceStatus::Disconnected;
        h_flex()
            .gap_2()
            .items_center()
            .child(div().text_xs().text_color(color).child(label))
            .when_some(self.waiting, |row, wait| {
                row.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(Spinner::new().small().color(cx.theme().warning))
                        .child(div().text_xs().text_color(cx.theme().warning).child(t(
                            match wait {
                                SignInWait::Sessions => "browser_sessions_checking",
                                SignInWait::Browser => "waiting_for_browser",
                            },
                        ))),
                )
            })
            .when(down && !waiting, |row| {
                row.children(self.on_sign_in.map(|on_sign_in| {
                    Button::new("status-sign-in")
                        .custom(style::soft_button(cx.theme().foreground, cx))
                        .xsmall()
                        .label(t("login"))
                        .on_click(move |event, window, cx| on_sign_in(event, window, cx))
                }))
            })
    }
}
