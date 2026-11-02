//! The one entity that follows the playhead: scrubbing, the clocks and the bar, so a tick redraws only them.

use components::{SeekBar, TrackClock};
use gpui_kit::base::slider::{SliderEvent, SliderState};
use gpui_kit::component::h_flex;
use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    Subscription, Window, div, rems,
};
use store::{PlayerModel, Playhead};

use crate::{fraction_state, played_fraction, seekable_duration_ms, track_duration_ms};

/// A thousandth of the track is finer than one pixel of bar in any window this app opens in.
const SEEK_STEP: f32 = 0.001;

/// What one surface draws of the playhead.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Layout {
    /// The clock either side of the bar, with the right one toggling to what is left.
    Clocked,
    /// The bar alone, for a surface that shows the reading elsewhere.
    BarOnly,
}

/// The seek slider and everything that moves with the playhead; the only observer of it.
pub(crate) struct SeekRow {
    state: Entity<SliderState>,
    layout: Layout,
    /// Drawn in the bar, so it follows the bar's size preset.
    scaled: bool,
    /// The fraction under the pointer while a drag runs; the release commits it.
    scrubbing: Option<f32>,
    _subscriptions: Vec<Subscription>,
}

impl SeekRow {
    pub(crate) fn new(
        layout: Layout,
        scaled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let start = played_fraction(PlayerModel::global(cx).read(cx));
        let state = cx.new(|_| fraction_state(SEEK_STEP, start));
        let subscriptions = vec![
            cx.subscribe_in(&state, window, Self::on_seek),
            cx.observe_in(&Playhead::global(cx), window, |this, _, window, cx| {
                this.sync(window, cx);
            }),
        ];
        Self {
            state,
            layout,
            scaled,
            scrubbing: None,
            _subscriptions: subscriptions,
        }
    }

    /// Carry the daemon's position into the slider the pointer is not holding.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.scrubbing.is_none() {
            let played = played_fraction(PlayerModel::global(cx).read(cx));
            self.state
                .update(cx, |state, cx| state.set_value(played, window, cx));
        }
        cx.notify();
    }

    /// What the clocks read: where the pointer is holding the bar, else where the daemon is.
    pub(crate) fn shown_ms(&self, position_ms: u64, total_ms: Option<u64>) -> u64 {
        match (self.scrubbing, total_ms) {
            (Some(fraction), Some(total)) => (f64::from(fraction) * total as f64) as u64,
            _ => position_ms,
        }
    }

    fn on_seek(
        &mut self,
        _: &Entity<SliderState>,
        event: &SliderEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            SliderEvent::Change(value) => self.scrubbing = Some(value.end()),
            SliderEvent::Release(value) => {
                let fraction = value.end();
                self.scrubbing = None;
                if let Some(total) = seekable_duration_ms(cx) {
                    let position = (f64::from(fraction) * total as f64).round() as u64;
                    PlayerModel::global(cx)
                        .update(cx, |player, cx| player.seek(position.min(total), cx));
                }
            }
        }
        cx.notify();
    }
}

impl Render for SeekRow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (position_ms, total_ms, played, loading, ranges, seekable) = {
            let player = PlayerModel::global(cx);
            let player = player.read(cx);
            (
                player.position_ms(),
                track_duration_ms(player),
                played_fraction(player),
                player.is_loading(),
                player.buffered().to_vec(),
                player.track().is_none_or(|track| track.seekable),
            )
        };
        let shown_ms = self.shown_ms(position_ms, total_ms);
        let scale = match self.scaled {
            true => store::layout(cx).player.size.scale(),
            false => 1.,
        };
        let bar = SeekBar::new("seek", &self.state)
            .played(self.scrubbing.unwrap_or(played))
            .total(total_ms)
            .buffered(&ranges)
            .loading(loading)
            .seekable(seekable)
            .scrubbing(self.scrubbing.is_some())
            .preview(true)
            .scale(scale);

        match self.layout {
            Layout::BarOnly => div().w_full().child(bar).into_any_element(),
            Layout::Clocked => h_flex()
                .w_full()
                .gap(rems(0.5 * scale))
                .items_center()
                .child(TrackClock::new(shown_ms, total_ms).scale(scale))
                .child(div().flex_1().min_w_0().child(bar))
                .child(
                    TrackClock::new(shown_ms, total_ms)
                        .id("seek-trailing")
                        .trailing()
                        .remaining_toggle(true)
                        .scale(scale),
                )
                .into_any_element(),
        }
    }
}

/// The reading the mini surface keeps beside the title, following the bar it belongs to.
pub(crate) struct PlayheadClock {
    seek: Entity<SeekRow>,
    _observe: Subscription,
}

impl PlayheadClock {
    pub(crate) fn new(seek: &Entity<SeekRow>, cx: &mut Context<Self>) -> Self {
        Self {
            seek: seek.clone(),
            _observe: cx.observe(seek, |_, _, cx| cx.notify()),
        }
    }
}

impl Render for PlayheadClock {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (position_ms, total_ms) = {
            let player = PlayerModel::global(cx);
            let player = player.read(cx);
            (player.position_ms(), track_duration_ms(player))
        };
        let shown_ms = self.seek.read(cx).shown_ms(position_ms, total_ms);
        TrackClock::new(shown_ms, total_ms).summary()
    }
}
