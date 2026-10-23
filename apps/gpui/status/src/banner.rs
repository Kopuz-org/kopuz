//! The daemon is not there: said in a strip across the chrome, with the page still behind it.

use components::format::{t, t_with};
use components::{Badge, BadgeTone, Chip, ChipTone, icons, style};
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Render,
    SharedString, Styled, Subscription, Window, div, px,
};
use link::ConnectionStatus;

const GLYPH: Pixels = px(12.);
// `bg-amber-500/15` and its edge: the connecting states are amber.
const FILL: f32 = 0.15;
const EDGE: f32 = 0.20;

/// The reconnect strip. It renders nothing until the daemon has first gone missing.
pub struct ConnectionBanner {
    // Cleared by the next status change, so the button cannot be pressed into spawning daemons.
    starting: bool,
    // The last thing said while offline, kept up while the strip fades out after a reconnect.
    last: Option<(ConnectionStatus, u32)>,
    _subscriptions: Vec<Subscription>,
}

impl ConnectionBanner {
    pub fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        let connection = link::Link::connection(cx);
        Self {
            starting: false,
            last: None,
            _subscriptions: vec![cx.observe(&connection, |this: &mut Self, _, cx| {
                this.starting = false;
                cx.notify();
            })],
        }
    }

    /// Whether the page behind should be dimmed, for whoever draws it.
    pub fn is_offline(cx: &gpui_kit::App) -> bool {
        !link::Link::connection(cx).read(cx).is_connected()
    }

    /// What the strip says: the first probe is not a failure, and a later one counts itself.
    fn line(status: &ConnectionStatus, attempts: u32) -> SharedString {
        match status {
            ConnectionStatus::Connected => SharedString::default(),
            ConnectionStatus::Connecting => t("connecting"),
            ConnectionStatus::Unreachable(_) if attempts <= 1 => t("connecting"),
            ConnectionStatus::Unreachable(_) => {
                t_with("reconnecting_attempt", &[("attempt", attempts.into())])
            }
            // Which of the two to rebuild is the only useful thing to say, so say it.
            ConnectionStatus::Mismatched { daemon, client } => t(match daemon < client {
                true => "daemon_older",
                false => "daemon_newer",
            }),
        }
    }
}

impl Render for ConnectionBanner {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (status, attempts) = {
            let connection = link::Link::connection(cx);
            let connection = connection.read(cx);
            (connection.status().clone(), connection.attempts())
        };
        if status != ConnectionStatus::Connected {
            self.last = Some((status, attempts));
        }
        let Some((status, attempts)) = self.last.clone() else {
            return div().into_any_element();
        };
        let reason = match &status {
            ConnectionStatus::Unreachable(error) => Some(store::human(error)),
            _ => None,
        };
        let line = Self::line(&status, attempts);
        let warning = cx.theme().warning;
        let starting = self.starting;
        h_flex()
            .w_full()
            .flex_shrink_0()
            .occlude()
            .items_center()
            .gap_2()
            .px_3()
            .py_1p5()
            .text_xs()
            .bg(warning.opacity(FILL))
            .border_b_1()
            .border_color(warning.opacity(EDGE))
            .text_color(warning)
            .child(
                icons::icon(icons::WARNING)
                    .size(GLYPH)
                    .flex_shrink_0()
                    .text_color(warning),
            )
            .child(div().flex_shrink_0().child(line))
            .children(reason.map(|reason| {
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(style::white(style::TEXT_MUTED, cx))
                    .child(reason)
                    .into_any_element()
            }))
            .child(div().flex_1().min_w_0())
            .child(
                Chip::new("connection-retry")
                    .label(t("retry_now"))
                    .tone(ChipTone::Ink(warning))
                    .on_click(|_, _, cx| {
                        link::Link::connection(cx).read(cx).retry_now();
                    }),
            )
            .map(|banner| match starting {
                true => banner.child(Badge::new(t("starting")).tone(BadgeTone::Muted)),
                false => banner.child(
                    Chip::new("connection-start")
                        .label(t("start_daemon"))
                        .tone(ChipTone::Ink(cx.theme().primary))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.starting = true;
                            cx.notify();
                            // Only logging this made users press again, spawning another daemon each time.
                            if let Err(error) = crate::spawn_daemon(cx) {
                                tracing::error!(%error, "could not start the daemon");
                                this.starting = false;
                                store::Notices::error(error.to_string(), cx);
                            }
                        })),
                ),
            })
            .into_any_element()
    }
}
