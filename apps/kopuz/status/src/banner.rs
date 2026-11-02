//! The daemon is not there: said in a strip across the chrome, with the page still behind it.

use components::format::{t, t_with};
use components::{Chip, ChipTone, icons, style};
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Render,
    SharedString, Styled, Subscription, Window, div, px,
};
use link::ConnectionStatus;

const GLYPH: Pixels = px(12.);
// `bg-amber-500/15` and its edge: the connecting states are amber.
const FILL: f32 = 0.15;
const EDGE: f32 = 0.20;

/// What the strip draws from the connection, kept up while it fades out after a reconnect.
#[derive(Clone)]
struct Said {
    status: ConnectionStatus,
    attempts: u32,
    starting: bool,
    blocked: bool,
}

/// The reconnect strip. It renders nothing until the daemon has first gone missing.
pub struct ConnectionBanner {
    last: Option<Said>,
    _subscriptions: Vec<Subscription>,
}

impl ConnectionBanner {
    pub fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        let connection = link::Link::connection(cx);
        Self {
            last: None,
            _subscriptions: vec![cx.observe(&connection, |_: &mut Self, _, cx| {
                cx.notify();
            })],
        }
    }

    /// Whether the page behind should be dimmed, for whoever draws it.
    pub fn is_offline(cx: &gpui_kit::App) -> bool {
        !link::Link::connection(cx).read(cx).is_connected()
    }

    /// Which of the two to rebuild is the only useful thing to say about a mismatch, so say it.
    pub(crate) fn mismatch(daemon: u32, client: u32) -> SharedString {
        t(match daemon < client {
            true => "daemon_older",
            false => "daemon_newer",
        })
    }

    /// What the strip says: the first probe is not a failure, and a later one counts itself.
    fn line(said: &Said) -> SharedString {
        match &said.status {
            ConnectionStatus::Connected => SharedString::default(),
            ConnectionStatus::Unreachable(_) if said.starting => t("starting"),
            ConnectionStatus::Connecting => t("connecting"),
            ConnectionStatus::Unreachable(_) if said.attempts <= 1 => t("connecting"),
            ConnectionStatus::Unreachable(_) => {
                t_with("reconnecting_attempt", &[("attempt", said.attempts.into())])
            }
            ConnectionStatus::Mismatched { daemon, client } => Self::mismatch(*daemon, *client),
        }
    }

    fn reason(said: &Said) -> Option<SharedString> {
        match &said.status {
            _ if said.blocked => Some(t("daemon_not_answering")),
            ConnectionStatus::Unreachable(error) => Some(store::human(error)),
            _ => None,
        }
    }
}

impl Render for ConnectionBanner {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let said = {
            let connection = link::Link::connection(cx);
            let connection = connection.read(cx);
            Said {
                status: connection.status().clone(),
                attempts: connection.attempts(),
                starting: connection.is_starting(),
                blocked: connection.is_blocked(),
            }
        };
        if said.status != ConnectionStatus::Connected {
            self.last = Some(said);
        }
        let Some(said) = self.last.clone() else {
            return div().into_any_element();
        };
        let reason = Self::reason(&said);
        let line = Self::line(&said);
        // A starting daemon answers or crashes the app; a retry would only wait on it again.
        let retry = (!said.starting).then(|| {
            Chip::new("connection-retry")
                .label(t("retry_now"))
                .tone(ChipTone::Ink(cx.theme().warning))
                .on_click(|_, _, cx| {
                    link::Link::connection(cx).read(cx).retry_now();
                })
        });
        let warning = cx.theme().warning;
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
            .children(retry)
            .into_any_element()
    }
}
