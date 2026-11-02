//! Startup: the mark drawing itself on until the daemon answers, and a way to retry one this app does not start.

use std::time::{Duration, Instant};

use components::format::t;
use gpui_kit::component::button::Button;
use gpui_kit::component::v_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, FontWeight, IntoElement, ParentElement as _, Pixels, Render, Styled, Task, Window,
    div, img, px,
};

use crate::logo;

/// How long the daemon has to answer before the screen offers a retry.
pub const DOWN_AFTER: Duration = Duration::from_secs(3);
/// The daemon-down panel's fade.
const FADE: Duration = Duration::from_millis(300);
/// How far the panel rises as it fades in.
const RISE: f32 = 6.;
const REASON_WIDTH: Pixels = px(420.);
// The startup screen has no theme yet, so its secondary line is the text colour, faded.
const REASON_OPACITY: f32 = 0.7;
const LOGO_WIDTH: Pixels = px(120.);
const LOGO_HEIGHT: Pixels = px(100.);

/// How far into its fade the daemon-down panel is, `elapsed` into the loader.
pub fn down_shown(elapsed: Duration, connected: bool, reduced: bool) -> f32 {
    if connected || elapsed < DOWN_AFTER {
        return 0.;
    }
    if reduced {
        return 1.;
    }
    ((elapsed - DOWN_AFTER).as_secs_f32() / FADE.as_secs_f32()).clamp(0., 1.)
}

/// Shown only at startup, until the daemon answers and setup's state is read.
pub struct Loader {
    started: Instant,
    _wake: Task<()>,
}

impl Loader {
    pub fn new(started: Instant, cx: &mut Context<Self>) -> Self {
        // Reduced motion paints no frames of its own, so the panel needs a nudge to appear.
        let wait = DOWN_AFTER.saturating_sub(started.elapsed());
        let wake = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |_, cx| cx.notify()).ok();
        });
        Self {
            started,
            _wake: wake,
        }
    }
}

impl Render for Loader {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = theme::startup();
        let reduced = cx.reduce_motion() || components::motion::span(FADE, cx).is_none();
        let connection = link::Link::connection(cx);
        let connection = connection.read(cx);
        let connected = connection.is_connected();
        let status = connection.status().clone();
        // Starting its own daemon ends in an answer or a crash saying why; anything else needs the user.
        let stuck = !link::Link::global(cx).starts_own()
            || connection.is_blocked()
            || matches!(status, link::ConnectionStatus::Mismatched { .. });
        let elapsed = self.started.elapsed();
        let (frame, opacity) = logo::pose(elapsed, reduced);
        let down = match stuck {
            true => down_shown(elapsed, connected, reduced),
            false => 0.,
        };
        let reason = match &status {
            _ if connection.is_blocked() => Some(t("daemon_not_answering")),
            link::ConnectionStatus::Mismatched { daemon, client } => {
                Some(crate::banner::ConnectionBanner::mismatch(*daemon, *client))
            }
            link::ConnectionStatus::Unreachable(error) => Some(store::human(error)),
            _ => None,
        };
        if !reduced {
            window.request_animation_frame();
        }
        let frames = logo::frames();
        let image = frames[frame.min(frames.len() - 1)].clone();
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_8()
            .bg(colors.background)
            .text_color(colors.text)
            .child(img(image).w(LOGO_WIDTH).h(LOGO_HEIGHT).opacity(opacity))
            .child(
                v_flex()
                    .items_center()
                    .gap_3()
                    .relative()
                    .top(px(RISE * (1. - down)))
                    .opacity(down)
                    .when(down > 0., |panel| {
                        panel
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(t("daemon_down_title")),
                            )
                            .children(reason.map(|reason| {
                                div()
                                    .max_w(REASON_WIDTH)
                                    .text_sm()
                                    .text_center()
                                    .opacity(REASON_OPACITY)
                                    .child(reason)
                            }))
                            .child(
                                Button::new("loader-retry")
                                    .outline()
                                    .label(t("retry"))
                                    .on_click(|_, _, cx| {
                                        link::Link::connection(cx).read(cx).retry_now();
                                        store::SetupModel::global(cx)
                                            .update(cx, |setup, cx| setup.read(cx));
                                    }),
                            )
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_daemon_gets_three_seconds_before_anything_is_said() {
        assert_eq!(down_shown(Duration::from_secs(2), false, false), 0.);
        assert_eq!(down_shown(DOWN_AFTER, false, false), 0.);
        assert_eq!(down_shown(DOWN_AFTER + FADE, false, false), 1.);
    }

    #[test]
    fn an_answer_or_reduced_motion_settles_it_at_once() {
        assert_eq!(down_shown(Duration::from_secs(10), true, false), 0.);
        assert_eq!(
            down_shown(DOWN_AFTER + Duration::from_millis(1), false, true),
            1.
        );
    }
}
