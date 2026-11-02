//! The performance HUD behind `ctrl-shift-p`: what the last second cost, drawn over the page.

use std::time::Duration;

use components::style;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Context, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    Render, SharedString, Styled, Task, Window, actions, div, px,
};

actions!(kopuz, [ToggleDebugOverlay]);

/// How often the HUD re-reads the counters; a frame's worth of numbers is noise, a second's is not.
const SAMPLE: Duration = Duration::from_secs(1);

const CARD_WIDTH: Pixels = px(208.);
const LABEL_ALPHA: f32 = 0.55;

/// What a view's `render` may cost before the heartbeat line is evidence of a bug (docs/V2.md §12).
const BUDGET_MS: f32 = 2.;

/// Reads the profiler on its own timer: an overlay that reports frame cost must not cost a frame.
pub struct DebugOverlay {
    open: bool,
    meter: profile::Meter,
    pulse: profile::Pulse,
    worst: Option<(&'static str, profile::SectionStat)>,
    _sampler: Option<Task<()>>,
}

impl DebugOverlay {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut overlay = Self {
            open: false,
            meter: profile::Meter::new(),
            pulse: profile::Pulse::default(),
            worst: None,
            _sampler: None,
        };
        // `KOPUZ_PERF_HUD=1` opens it from the start, which is how a smoke run gets a HUD.
        if std::env::var_os("KOPUZ_PERF_HUD").is_some() {
            overlay.toggle(cx);
        }
        overlay
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Opening starts the sampler; closing stops it, so a shut HUD costs nothing at idle.
    pub fn toggle(&mut self, cx: &mut Context<Self>) {
        self.open = !self.open;
        if self.open {
            self.meter = profile::Meter::new();
            self.pulse = self.meter.sample();
            self.worst = profile::worst(1).into_iter().next();
            self._sampler = Some(Self::sample(cx));
        } else {
            self._sampler = None;
        }
        cx.notify();
    }

    fn sample(cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(SAMPLE).await;
                // One rebuild a second, which the reading itself then accounts for.
                let sampled = this.update(cx, |this, cx| {
                    this.pulse = this.meter.sample();
                    this.worst = profile::worst(1).into_iter().next();
                    cx.notify();
                });
                if sampled.is_err() {
                    return;
                }
            }
        })
    }

    fn row(&self, label: &'static str, value: SharedString, over: bool, cx: &App) -> AnyElement {
        h_flex()
            .justify_between()
            .gap_2()
            .child(div().text_color(style::white(LABEL_ALPHA, cx)).child(label))
            .child(
                div()
                    .text_color(match over {
                        true => cx.theme().danger,
                        false => cx.theme().foreground,
                    })
                    .child(value),
            )
            .into_any_element()
    }
}

impl Render for DebugOverlay {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return div().into_any_element();
        }
        let pulse = self.pulse;
        let (worst_view, worst_ms) = match &self.worst {
            Some((name, stat)) => (SharedString::from(*name), stat.worst_us as f32 / 1000.),
            None => (SharedString::new_static("-"), 0.),
        };
        v_flex()
            .absolute()
            .top_4()
            .right_4()
            .w(CARD_WIDTH)
            .occlude()
            .gap_1()
            .p_2()
            .rounded(style::radius_lg())
            .bg(cx.theme().popover)
            .border_1()
            .border_color(style::white(style::BORDER, cx))
            .shadow_lg()
            .text_size(style::text_11())
            .child(
                div()
                    .text_color(style::white(LABEL_ALPHA, cx))
                    .child("perf · ctrl-shift-p"),
            )
            .child(self.row(
                "rebuilds/s",
                format!("{:.0}", pulse.rebuilds_per_s).into(),
                false,
                cx,
            ))
            .child(self.row(
                "notifies/s",
                format!("{:.0}", pulse.notifies_per_s).into(),
                false,
                cx,
            ))
            .child(self.row(
                "elements/frame",
                format!("{:.0}", pulse.elements_per_frame).into(),
                false,
                cx,
            ))
            .child(self.row("worst view", worst_view, worst_ms > BUDGET_MS, cx))
            .child(self.row(
                "worst render",
                format!("{worst_ms:.1}ms").into(),
                worst_ms > BUDGET_MS,
                cx,
            ))
            .child(self.row(
                "covers live",
                pulse.counters.artwork_live.to_string().into(),
                false,
                cx,
            ))
            .child(self.row(
                "allocated",
                format!("{}MB", pulse.allocated_mb).into(),
                false,
                cx,
            ))
            .into_any_element()
    }
}
