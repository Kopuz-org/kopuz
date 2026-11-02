//! What a view draws instead of content: nothing there, a read that failed, a read still on its way, and the inline error banner.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, ClickEvent, IntoElement, ParentElement as _, RenderOnce, SharedString, Styled, Window,
    div, px,
};

use crate::format::t;
use crate::{icons, motion, style};

/// How much room a state takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum StateSize {
    /// Fills the page or panel it stands in for, centred.
    #[default]
    Page,
    /// A short centred line inside a popover, a list or a settings card.
    Compact,
}

/// `max-w-[420px]`, so a long reason wraps instead of running the width of the page.
const TEXT_WIDTH: f32 = 420.;

type Act = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// What a view shows instead of rows: nothing found, nothing configured, or a read that failed.
#[derive(IntoElement)]
pub struct EmptyState {
    title: SharedString,
    detail: Option<SharedString>,
    icon: Option<&'static str>,
    danger: bool,
    size: StateSize,
    action: Option<(SharedString, Option<&'static str>, Act)>,
}

impl EmptyState {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            detail: None,
            icon: None,
            danger: false,
            size: StateSize::Page,
            action: None,
        }
    }

    /// A read that failed, said plainly, with the button that runs it again.
    pub fn failed(
        title: impl Into<SharedString>,
        retry: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        let mut state = Self::new(title).icon(icons::WARNING).danger();
        state.action = Some((t("retry_now"), Some(icons::REFRESH), Rc::new(retry)));
        state
    }

    /// Offline mode keeps a remote library to the songs with a copy here, and there are none; the button leaves offline mode.
    pub fn offline() -> Self {
        Self::new(t("library_empty_offline"))
            .icon(icons::CLOUD_OFF)
            .detail(t("library_empty_offline_detail"))
            .action(t("offline_mode_turn_off"), |_, _, cx| {
                store::SourcesModel::global(cx)
                    .update(cx, |sources, cx| sources.set_offline(false, cx))
            })
    }

    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }

    /// The glyph in the theme's danger colour, for a state that is a failure.
    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }

    pub fn compact(mut self) -> Self {
        self.size = StateSize::Compact;
        self
    }

    /// The one button that gets out of the state, such as signing in or clearing a filter.
    pub fn action(
        mut self,
        label: impl Into<SharedString>,
        act: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.action = Some((label.into(), None, Rc::new(act)));
        self
    }
}

impl RenderOnce for EmptyState {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let fade = motion::enter(motion::id("empty", &self.title), motion::MEDIUM, window, cx);
        let glyph = match self.danger {
            true => cx.theme().danger,
            false => style::white(style::TEXT_FAINT, cx),
        };
        let compact = self.size == StateSize::Compact;
        let icon = match compact {
            true => self.icon,
            false => self.icon.or(Some(icons::MUSIC)),
        };
        let action_id = motion::id("empty-action", &self.title);
        v_flex()
            .items_center()
            .justify_center()
            .text_center()
            .opacity(fade)
            .map(|column| match compact {
                true => column
                    .w_full()
                    .gap_2()
                    .px_3()
                    .py_6()
                    .text_xs()
                    .text_color(style::white(style::TEXT_MUTED, cx)),
                false => column
                    .size_full()
                    .gap_1()
                    .px_6()
                    .py_12()
                    .text_color(style::white(style::TEXT_SUBTLE, cx)),
            })
            .when_some(icon, |column, icon| {
                column.child(
                    Icon::default()
                        .path(icon)
                        .map(|glyph| match compact {
                            true => glyph.size(px(16.)),
                            false => glyph.size(px(36.)).mb_4(),
                        })
                        .text_color(glyph),
                )
            })
            .child(
                div()
                    .max_w(px(TEXT_WIDTH))
                    .when(!compact, |title| title.text_base())
                    .child(self.title),
            )
            .when_some(self.detail, |column, detail| {
                column.child(
                    div()
                        .max_w(px(TEXT_WIDTH))
                        .when(!compact, |detail| detail.text_sm())
                        .text_color(style::white(style::TEXT_DIM, cx))
                        .child(detail),
                )
            })
            .when_some(self.action, |column, (label, glyph, act)| {
                column.child(
                    div().mt_5().child(
                        Button::new(action_id)
                            .primary()
                            .when(compact, |button| button.small())
                            .when_some(glyph, |button, glyph| button.icon(icons::icon(glyph)))
                            .label(label)
                            .on_click(move |event, window, cx| act(event, window, cx)),
                    ),
                )
            })
    }
}

/// The spinner a read that has not answered yet stands behind, optionally saying what it waits on.
#[derive(IntoElement, Default)]
pub struct Loading {
    label: Option<SharedString>,
    size: StateSize,
}

impl Loading {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// A small spinner beside its label, for a list still growing under what it already shows.
    pub fn compact(mut self) -> Self {
        self.size = StateSize::Compact;
        self
    }
}

impl RenderOnce for Loading {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let fade = motion::enter("loading", motion::MEDIUM, window, cx);
        let ink = style::white(style::TEXT_MUTED, cx);
        match self.size {
            StateSize::Page => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .py_12()
                .opacity(fade)
                .child(
                    Spinner::new()
                        .large()
                        .color(style::white(style::TEXT_FAINT, cx)),
                )
                .when_some(self.label, |column, label| {
                    column.child(div().text_sm().text_color(ink).child(label))
                })
                .into_any_element(),
            StateSize::Compact => h_flex()
                .w_full()
                .items_center()
                .justify_center()
                .gap_3()
                .py_6()
                .opacity(fade)
                .text_xs()
                .text_color(ink)
                .child(Spinner::new().small())
                .children(self.label)
                .into_any_element(),
        }
    }
}

/// `.popup .error`: red text in a red box, the way a form or dialog reports what it was refused.
#[derive(IntoElement)]
pub struct ErrorBanner {
    message: SharedString,
    icon: bool,
    reveal: f32,
}

impl ErrorBanner {
    pub fn new(message: impl Into<SharedString>) -> Self {
        Self {
            message: message.into(),
            icon: false,
            reveal: 1.,
        }
    }

    /// A warning glyph before the message, for a refusal that stops what the dialog was for.
    pub fn icon(mut self) -> Self {
        self.icon = true;
        self
    }

    /// How far it has faded in, for a banner whose owner keeps the channel across its absence.
    pub fn reveal(mut self, reveal: f32) -> Self {
        self.reveal = reveal;
        self
    }
}

impl RenderOnce for ErrorBanner {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let danger = cx.theme().danger;
        h_flex()
            .w_full()
            .gap_2()
            .items_start()
            .px_3()
            .py_2()
            .rounded(style::radius_lg())
            .border_1()
            .border_color(danger.opacity(0.3))
            .bg(danger.opacity(0.1))
            .text_sm()
            .text_color(danger)
            .when(self.reveal < 1., |banner| banner.opacity(self.reveal))
            .when(self.icon, |banner| {
                banner.child(Icon::default().path(icons::WARNING).small())
            })
            .child(div().flex_1().min_w_0().child(self.message))
    }
}

/// Why one field is wrong, under that field rather than at the foot of the form.
#[derive(IntoElement)]
pub struct FieldProblem(SharedString);

impl FieldProblem {
    pub fn new(message: impl Into<SharedString>) -> Self {
        Self(message.into())
    }
}

impl RenderOnce for FieldProblem {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        div().text_xs().text_color(cx.theme().danger).child(self.0)
    }
}
