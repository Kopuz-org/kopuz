//! The banners over the routed page: the scan line, the playback error, and the daemon's absence.

use api::{JobKind, JobProgress};
use components::format::t;
use components::{JobLine, ProgressBar, icons, motion, style};
use gpui_kit::base::motion::{Easing, Presence, Transition};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, Render, SharedString, StatefulInteractiveElement as _, Styled,
    Subscription, Window, div, px,
};
mod hint;

use status::{ConnectionBanner, NetworkBanner, Toasts};
use store::{JobsModel, PlayerModel};

// TODO(parity): the update banner needs GitHub, which is outside the daemon seam (§3.1).

const SCAN_BAR: Pixels = px(2.);
const SCAN_ROW_PY: Pixels = px(3.);
const TINY_ICON: Pixels = px(9.);
const SMALL_ICON: Pixels = px(12.);

// `bg-black/30` behind the scan line, and the two off-scale text alphas it uses.
const SCAN_SCRIM: f32 = 0.30;
const SCAN_GLYPH: f32 = 0.30;
const SCAN_TEXT: f32 = 0.35;
// `text-white/40` on the dismiss ✕, and `bg-rose-500/15` behind the error banner.
const DISMISS_TEXT: f32 = 0.40;
const BANNER_FILL: f32 = 0.15;
const BANNER_EDGE: f32 = 0.20;
// The page stays readable behind a disconnect, but not mistakable for live.
const OFFLINE_SCRIM: f32 = 0.45;

/// Drawn over the routed page, so it paints nothing until it has something to say.
pub struct Overlays {
    // The error the user closed, so the banner stays gone until the daemon reports a different one.
    dismissed_error: Option<String>,
    banner: Entity<ConnectionBanner>,
    network: Entity<NetworkBanner>,
    // The dimmed page and the reconnect strip come and go together, fading rather than cutting.
    offline_fade: motion::Fade,
    // Held, not drawn: toasts must keep arriving in compact mode, where nothing here renders.
    _toasts: Entity<Toasts>,
    _subscriptions: Vec<Subscription>,
}

impl Overlays {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let jobs = JobsModel::global(cx);
        let player = PlayerModel::global(cx);
        let banner = cx.new(|cx| ConnectionBanner::new(window, cx));
        let network = cx.new(|cx| NetworkBanner::new(window, cx));
        let subscriptions = vec![
            cx.observe(&jobs, |_, _, cx| cx.notify()),
            cx.observe(&player, |_, _, cx| cx.notify()),
            cx.observe(&banner, |_, _, cx| cx.notify()),
            cx.observe(&network, |_, _, cx| cx.notify()),
            cx.observe(&store::SetupModel::global(cx), |_, _, cx| cx.notify()),
            cx.observe(&store::LayoutModel::global(cx), |_, _, cx| cx.notify()),
        ];
        Self {
            dismissed_error: None,
            banner,
            network,
            offline_fade: motion::Fade::hidden(),
            _toasts: cx.new(|cx| Toasts::new(window, cx)),
            _subscriptions: subscriptions,
        }
    }

    /// Switching source clears the error it belonged to, so what is here is about what plays now.
    fn playback_error(&self, cx: &App) -> Option<SharedString> {
        let message = store::human_body(PlayerModel::global(cx).read(cx).error()?);
        if self.dismissed_error == Self::occurrence(cx) {
            return None;
        }
        Some(message)
    }

    /// A dismissal is keyed on the failure and its track, so dismissing one never silences the next.
    fn occurrence(cx: &App) -> Option<String> {
        let player = PlayerModel::global(cx);
        let player = player.read(cx);
        let message = &player.error()?.message;
        let track = player.track().map(|track| track.key.as_str()).unwrap_or("");
        Some(format!("{track}\u{1}{message}"))
    }

    fn scan_banner(
        &self,
        progress: &JobProgress,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = progress.id.clone();
        v_flex()
            .w_full()
            .flex_shrink_0()
            .occlude()
            .opacity(appear("scan-banner", window, cx))
            .child(ProgressBar::new("scan-progress", None).height(SCAN_BAR))
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .px_3()
                    .py(SCAN_ROW_PY)
                    .bg(style::black(SCAN_SCRIM, cx))
                    .border_b_1()
                    .border_color(style::white(style::BORDER_SOFT, cx))
                    .text_size(style::text_10())
                    .text_color(style::white(SCAN_TEXT, cx))
                    .child(
                        icons::icon(icons::DISC)
                            .size(TINY_ICON)
                            .flex_shrink_0()
                            .text_color(style::white(SCAN_GLYPH, cx)),
                    )
                    .child(
                        div().flex_1().min_w_0().child(
                            JobLine::new(progress)
                                .idle_phase(t("scanning_library"))
                                .stopping(JobsModel::global(cx).read(cx).is_stopping(&progress.id))
                                // A scan could not be stopped, though a download could, and both are jobs.
                                .on_stop(move |_, cx| {
                                    let id = id.clone();
                                    JobsModel::global(cx)
                                        .update(cx, |jobs, cx| jobs.cancel(id, cx));
                                }),
                        ),
                    ),
            )
            .into_any_element()
    }

    fn error_banner(
        &self,
        message: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let danger = cx.theme().danger;
        let (over, pointer) = motion::hovered("dismiss-error-hover", window, cx);
        let dismiss = motion::amount(
            "dismiss-error-alpha",
            if over { 1. } else { 0.5 },
            motion::FAST,
            window,
            cx,
        );
        h_flex()
            .w_full()
            .flex_shrink_0()
            .occlude()
            .opacity(appear("error-banner", window, cx))
            .items_center()
            .justify_between()
            .gap_3()
            .px_4()
            .py_2()
            .text_sm()
            .bg(danger.opacity(BANNER_FILL))
            .border_b_1()
            .border_color(danger.opacity(BANNER_EDGE))
            .text_color(danger)
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .items_center()
                    .child(icons::icon(icons::WARNING).size(SMALL_ICON).flex_shrink_0())
                    .child(div().flex_1().min_w_0().child(message.clone())),
            )
            .child(
                div()
                    .id("dismiss-playback-error")
                    .flex()
                    .flex_shrink_0()
                    .p_1()
                    .cursor_pointer()
                    .opacity(dismiss)
                    .text_color(style::white(DISMISS_TEXT, cx))
                    .on_hover(motion::track(pointer))
                    .tooltip(|window, cx| Tooltip::new(t("close")).build(window, cx))
                    .child(icons::icon(icons::CLOSE).size(SMALL_ICON))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.dismissed_error = Self::occurrence(cx);
                        cx.notify();
                    })),
            )
            .into_any_element()
    }
}

/// An overlay only exists while it has something to say, so its entrance fade starts from nothing.
fn appear(id: &'static str, window: &mut Window, cx: &mut App) -> f32 {
    Presence::new(id, true)
        .transition(Transition::new(motion::MEDIUM).easing(Easing::EaseOut))
        .sample(window, cx)
        .progress
}

impl Render for Overlays {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let scan = JobsModel::global(cx)
            .read(cx)
            .progress(JobKind::Scan)
            .cloned();
        let error = self.playback_error(cx);
        let lost = ConnectionBanner::is_offline(cx);
        let dimmed = self.offline_fade.sample(lost, motion::MEDIUM, window, cx);
        let offline = dimmed > 0.;
        let network = !lost && self.network.read(cx).is_showing(cx);
        let tips = match lost {
            true => None,
            false => hint::card(window, cx),
        };
        if scan.is_none() && error.is_none() && !offline && !network && tips.is_none() {
            return div().into_any_element();
        }

        let scan = scan.map(|progress| self.scan_banner(&progress, window, cx));
        let error = error.map(|message| self.error_banner(message, window, cx));
        let network = network.then(|| {
            div()
                .w_full()
                .opacity(appear("network-banner", window, cx))
                .child(self.network.clone())
        });
        v_flex()
            .absolute()
            .inset_0()
            // The page stays where it was, dimmed, rather than being replaced by a status screen.
            .when(offline, |layer| {
                layer.child(
                    div()
                        .absolute()
                        .inset_0()
                        .occlude()
                        .bg(style::black(OFFLINE_SCRIM * dimmed, cx)),
                )
            })
            .child(
                v_flex()
                    .w_full()
                    .when(offline, |column| {
                        column.child(div().w_full().opacity(dimmed).child(self.banner.clone()))
                    })
                    .children(network)
                    .children(scan)
                    .children(error),
            )
            .children(tips)
            .into_any_element()
    }
}
