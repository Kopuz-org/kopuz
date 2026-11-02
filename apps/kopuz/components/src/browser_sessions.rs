//! The browser profiles on this machine already signed in to a service, each one press from being used.

use std::rc::Rc;

use api::BrowserSession;
use gpui_kit::component::button::Button;
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, ElementId, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    RenderOnce, SharedString, Styled, Window, div, px,
};

use crate::format::{t, t_with};
use crate::{icons, style};

/// Lucide's account glyph, which `icons` has no name for yet.
const ACCOUNT: &str = "icons/circle-user-round.svg";

type OnUse = Rc<dyn Fn(BrowserSession, &mut Window, &mut App)>;

/// `None` is still looking: the daemon checks each profile with the service first, which takes a few seconds.
#[derive(IntoElement)]
pub struct BrowserSessionList {
    id: ElementId,
    sessions: Option<Vec<BrowserSession>>,
    importing: Option<String>,
    disabled: bool,
    on_use: Option<OnUse>,
}

impl BrowserSessionList {
    pub fn new(id: impl Into<ElementId>, sessions: Option<Vec<BrowserSession>>) -> Self {
        Self {
            id: id.into(),
            sessions,
            importing: None,
            disabled: false,
            on_use: None,
        }
    }

    /// The session being taken now, which reads as working while every other row waits.
    pub fn importing(mut self, importing: Option<String>) -> Self {
        self.importing = importing;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_use(
        mut self,
        on_use: impl Fn(BrowserSession, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_use = Some(Rc::new(on_use));
        self
    }
}

/// "Brave · Default", the profile as the browser names it.
pub fn session_label(session: &BrowserSession) -> SharedString {
    t_with(
        "browser_session_name",
        &[
            ("browser", session.browser.clone().into()),
            ("profile", session.profile.clone().into()),
        ],
    )
}

fn row(cx: &App) -> gpui_kit::Div {
    h_flex()
        .w_full()
        .gap_3()
        .items_center()
        .px_3()
        .py_2p5()
        .rounded(style::radius_lg())
        .border_1()
        .border_color(style::white(style::BORDER, cx))
        .bg(style::white(style::RAISED, cx))
}

impl RenderOnce for BrowserSessionList {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let looking = self.sessions.is_none();
        // Nothing signed in anywhere is no news; the ordinary sign-in is the way on.
        if self.sessions.as_ref().is_some_and(Vec::is_empty) {
            return div().id(self.id).into_any_element();
        }
        let busy = self.importing.is_some() || self.disabled;
        let heading = div()
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(style::white(style::TEXT_MUTED, cx))
            .child(match looking {
                true => t("browser_sessions_checking"),
                false => t("browser_sessions_heading"),
            });
        let glyph = |cx: &App| {
            icons::icon(ACCOUNT)
                .size(px(16.))
                .flex_shrink_0()
                .text_color(style::white(style::TEXT_MUTED, cx))
        };
        v_flex()
            .id(self.id)
            .w_full()
            .gap_2()
            .child(heading)
            .when(looking, |list| {
                list.child(
                    row(cx).child(glyph(cx)).child(
                        div()
                            .h(px(10.))
                            .w(px(160.))
                            .rounded_full()
                            .bg(style::white(style::RAISED, cx)),
                    ),
                )
            })
            .children(self.sessions.into_iter().flatten().map(|session| {
                let working = self.importing.as_deref() == Some(session.id.as_str());
                let on_use = self.on_use.clone();
                let chosen = session.clone();
                row(cx)
                    .child(glyph(cx))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_sm().truncate().child(session_label(&session)))
                            .children(session.account.clone().map(|account| {
                                div()
                                    .text_xs()
                                    .truncate()
                                    .text_color(style::white(style::TEXT_MUTED, cx))
                                    .child(account)
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("use-session-{}", session.id)))
                            .small()
                            .outline()
                            .label(match working {
                                true => t("browser_session_importing"),
                                false => t("browser_session_use"),
                            })
                            .loading(working)
                            .disabled(busy)
                            .on_click(move |_, window, cx| {
                                if let Some(on_use) = &on_use {
                                    on_use(chosen.clone(), window, cx);
                                }
                            }),
                    )
            }))
            .when(!looking, |list| {
                list.child(
                    div()
                        .text_xs()
                        .text_color(style::white(style::TEXT_MUTED, cx))
                        .child(t("browser_session_help")),
                )
            })
            .into_any_element()
    }
}
