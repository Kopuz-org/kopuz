//! Kopuz's transitions in GPUI terms; `gpui_base::motion` does the work and honours reduce-motion for us.

use std::time::{Duration, Instant};

use gpui_kit::base::animation::Lerp;
use gpui_kit::base::motion::{Easing, Transition, transition};
use gpui_kit::{App, ElementId, Hsla, Pixels, Rgba, Window};

/// Tailwind's bare `transition-*`, which is what most Kopuz hovers carry.
pub const FAST: Duration = Duration::from_millis(150);
/// `duration-200`.
pub const MEDIUM: Duration = Duration::from_millis(200);
/// `duration-300`, the carousels and the larger reveals.
pub const SLOW: Duration = Duration::from_millis(300);

/// How long a tween may take once the motion preference has had its say; `None` means snap.
pub fn span(duration: Duration, cx: &App) -> Option<Duration> {
    scaled(duration, false, store::layout(cx).motion.scale())
}

/// A colour on its way to `target`: `transition-colors` for one property.
///
/// `id` must be stable and unique within the element, since it keys the channel.
pub fn color(id: impl Into<ElementId>, target: Hsla, window: &mut Window, cx: &mut App) -> Hsla {
    color_over(id, target, FAST, window, cx)
}

/// The same over `duration`, for a `duration-300` hover or a slower fade.
pub fn color_over(
    id: impl Into<ElementId>,
    target: Hsla,
    duration: Duration,
    window: &mut Window,
    cx: &mut App,
) -> Hsla {
    let Some(duration) = span(duration, cx) else {
        return target;
    };
    let mixed = transition(
        id.into(),
        Mixed(target.into()),
        Transition::new(duration).easing(Easing::EaseInOut),
        window,
        cx,
    );
    mixed.0.into()
}

/// A colour eased channel by channel in RGB; gpui's `Hsla` tween lerps hue, so red to orange sweeps the whole wheel.
#[derive(Clone, Copy, PartialEq)]
struct Mixed(Rgba);

impl Lerp for Mixed {
    fn lerp(&self, target: &Self, t: f32) -> Self {
        let at = |from: f32, to: f32| from + (to - from) * t;
        let (from, to) = (self.0, target.0);
        Mixed(Rgba {
            r: at(from.r, to.r),
            g: at(from.g, to.g),
            b: at(from.b, to.b),
            a: at(from.a, to.a),
        })
    }
}

/// The accent `surface` wears, eased over a track change; `base` is what the theme would paint.
pub fn accent(surface: theme::Surface, base: Hsla, window: &mut Window, cx: &mut App) -> Hsla {
    let target = theme::accent_for(surface, window, cx).unwrap_or(base);
    color_over(
        id("accent", surface.key()),
        target,
        theme::ACCENT_TWEEN,
        window,
        cx,
    )
}

/// A length on its way to `target`: the cover zoom a `group-hover:scale-105` reads as.
pub fn length(
    id: impl Into<ElementId>,
    target: Pixels,
    duration: Duration,
    window: &mut Window,
    cx: &mut App,
) -> Pixels {
    let Some(duration) = span(duration, cx) else {
        return target;
    };
    transition(
        id.into(),
        target,
        Transition::new(duration).easing(Easing::EaseOut),
        window,
        cx,
    )
}

/// A bare number on its way to `target`, for an opacity or a progress fraction.
pub fn amount(
    id: impl Into<ElementId>,
    target: f32,
    duration: Duration,
    window: &mut Window,
    cx: &mut App,
) -> f32 {
    let Some(duration) = span(duration, cx) else {
        return target;
    };
    transition(
        id.into(),
        target,
        Transition::new(duration).easing(Easing::EaseInOut),
        window,
        cx,
    )
}

/// A cover's hovered size: `scale-105` over the frame it sits in, clipped by `overflow_hidden`.
pub fn zoom(
    id: impl Into<ElementId>,
    base: Pixels,
    hovered: bool,
    window: &mut Window,
    cx: &mut App,
) -> Pixels {
    zoom_over(id, base, hovered, SLOW, window, cx)
}

/// The same over `duration`, for the covers upstream marks `duration-500`.
pub fn zoom_over(
    id: impl Into<ElementId>,
    base: Pixels,
    hovered: bool,
    duration: Duration,
    window: &mut Window,
    cx: &mut App,
) -> Pixels {
    let target = if hovered { base * 1.05 } else { base };
    length(id, target, duration, window, cx)
}

/// `duration-500`, which several cover zooms upstream use.
pub const SLOWEST: Duration = Duration::from_millis(500);

/// A stable motion id from a prefix and a domain key, which is what every channel wants.
pub fn id(prefix: &str, key: impl std::fmt::Display) -> ElementId {
    ElementId::Name(format!("{prefix}-{key}").into())
}

/// The same for a row of a list, keyed by slot: a `NamedInteger` allocates nothing per frame.
pub fn row_id(prefix: &'static str, index: usize) -> ElementId {
    ElementId::from((prefix, index))
}

/// One of an element's channels, keyed off an id it already holds: a clone, not a `format!`.
pub fn channel(key: &gpui_kit::SharedString, channel: u64) -> ElementId {
    ElementId::NamedInteger(key.clone(), channel)
}

/// The incoming page's rise, and the palette's: far enough to read as arriving, not as sliding.
pub const RISE: Pixels = gpui_kit::px(8.);
/// A popup's drop, as gpui-component's own Select popup takes it.
pub const NUDGE: Pixels = gpui_kit::px(4.);
/// A menu's entrance, a hair quicker than a hover.
pub const MENU: Duration = Duration::from_millis(120);
/// A glyph swapping for another in place, such as play for pause.
pub const GLYPH: Duration = Duration::from_millis(100);

/// The tween length once both reduced-motion settings have had their say; `None` means snap.
pub fn live(duration: Duration, cx: &App) -> Option<Duration> {
    scaled(
        duration,
        cx.reduce_motion(),
        store::layout(cx).motion.scale(),
    )
}

/// `duration` under the two settings: `reduce_animations` snaps, and the layout's motion scales or snaps.
pub fn scaled(duration: Duration, reduce: bool, scale: f32) -> Option<Duration> {
    (!reduce && scale > 0.).then(|| duration.mul_f32(scale))
}

/// Ease-out cubic progress `elapsed` into `duration`, settled at 1; a zero duration is already there.
pub fn progress(elapsed: Duration, duration: Duration) -> f32 {
    if duration.is_zero() {
        return 1.;
    }
    let linear = (elapsed.as_secs_f32() / duration.as_secs_f32()).clamp(0., 1.);
    1. - (1. - linear).powi(3)
}

/// A fade between hidden and shown that the owning view keeps, so an unmount and remount does not replay it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fade {
    shown: bool,
    from: f32,
    since: Option<Instant>,
    duration: Duration,
}

impl Default for Fade {
    fn default() -> Self {
        Self::hidden()
    }
}

impl Fade {
    pub const fn hidden() -> Self {
        Self {
            shown: false,
            from: 0.,
            since: None,
            duration: Duration::ZERO,
        }
    }

    pub const fn shown() -> Self {
        Self {
            shown: true,
            from: 1.,
            since: None,
            duration: Duration::ZERO,
        }
    }

    fn target(&self) -> f32 {
        if self.shown { 1. } else { 0. }
    }

    /// Where the fade stands at `now`, from wherever it last turned.
    pub fn value_at(&self, now: Instant) -> f32 {
        match self.since {
            None => self.target(),
            Some(since) => {
                let eased = progress(now.saturating_duration_since(since), self.duration);
                self.from + (self.target() - self.from) * eased
            }
        }
    }

    /// Whether the fade has reached its target by `now`.
    pub fn settled_at(&self, now: Instant) -> bool {
        self.since
            .is_none_or(|since| now.saturating_duration_since(since) >= self.duration)
    }

    /// Head for `shown` from wherever the fade stands, so a reversal mid-way never jumps.
    pub fn turn_at(&mut self, shown: bool, now: Instant, duration: Duration) {
        if shown == self.shown {
            return;
        }
        self.from = self.value_at(now);
        self.shown = shown;
        self.since = Some(now);
        self.duration = duration;
    }

    /// Land on `shown` at once, for a reduced-motion snap.
    pub fn snap(&mut self, shown: bool) {
        *self = if shown { Self::shown() } else { Self::hidden() };
    }

    pub fn is_shown(&self) -> bool {
        self.shown
    }

    /// Whether any of it is on screen yet or still: shown, or fading out.
    pub fn is_visible(&self) -> bool {
        self.shown || self.value_at(Instant::now()) > 0.
    }

    /// Head for `shown` over `duration` and say where the fade stands, asking for frames until it lands.
    pub fn sample(
        &mut self,
        shown: bool,
        duration: Duration,
        window: &mut Window,
        cx: &App,
    ) -> f32 {
        let Some(duration) = live(duration, cx) else {
            self.snap(shown);
            return self.target();
        };
        let now = Instant::now();
        self.turn_at(shown, now, duration);
        if !self.settled_at(now) {
            window.request_animation_frame();
        }
        self.value_at(now)
    }
}

/// Fades in on the frame an element first renders, and again on every remount: keep a [`Fade`] where that is wrong.
pub fn enter(
    id: impl Into<ElementId>,
    duration: Duration,
    window: &mut Window,
    cx: &mut App,
) -> f32 {
    let id: ElementId = id.into();
    // Without a tween there is nothing to fade, and the blank first frame would just flicker.
    if span(duration, cx).is_none() {
        return 1.;
    }
    let shown = window.use_keyed_state(
        ElementId::Name(format!("{id:?}-entered").into()),
        cx,
        |_, _| false,
    );
    let entered = *shown.read(cx);
    if !entered {
        shown.update(cx, |shown, cx| {
            *shown = true;
            cx.notify();
        });
    }
    amount(id, if entered { 1. } else { 0. }, duration, window, cx)
}

/// `scale-105`, the square cards.
pub const ZOOM_SM: f32 = 1.05;
/// `scale-110`, the round artist card.
pub const ZOOM_LG: f32 = 1.10;

/// A cover's hovered size at an explicit factor, for the cards that zoom further.
pub fn zoom_by(
    id: impl Into<ElementId>,
    base: Pixels,
    factor: f32,
    hovered: bool,
    duration: Duration,
    window: &mut Window,
    cx: &mut App,
) -> Pixels {
    let target = if hovered { base * factor } else { base };
    length(id, target, duration, window, cx)
}

/// Hover as a value a tween can read; `.hover()` refines a style instead of reporting the state.
pub fn hovered(
    id: impl Into<ElementId>,
    window: &mut Window,
    cx: &mut App,
) -> (bool, gpui_kit::Entity<bool>) {
    let state = window.use_keyed_state(id.into(), cx, |_, _| false);
    let hovered = *state.read(cx);
    (hovered, state)
}

/// The `on_hover` listener that keeps a [`hovered`] flag in step with the pointer.
pub fn track(
    state: gpui_kit::Entity<bool>,
) -> impl Fn(&bool, &mut Window, &mut App) + 'static + use<> {
    move |over, _, cx| {
        let over = *over;
        state.update(cx, |flag, cx| {
            if *flag != over {
                *flag = over;
                cx.notify();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: fn(u64) -> Duration = Duration::from_millis;

    #[test]
    fn progress_eases_out_from_nothing_to_settled() {
        assert_eq!(progress(MS(0), FAST), 0.);
        assert_eq!(progress(FAST, FAST), 1.);
        assert_eq!(progress(MS(900), FAST), 1.);
        let half = progress(MS(75), FAST);
        assert!(
            half > 0.5 && half < 1.,
            "ease-out is past halfway at half time: {half}"
        );
        let mut last = 0.;
        for at in 0..=150 {
            let now = progress(MS(at), FAST);
            assert!(now >= last, "never backs up");
            last = now;
        }
    }

    #[test]
    fn a_colour_eases_the_short_way_rather_than_round_the_wheel() {
        let red = Mixed(gpui_kit::hsla(350. / 360., 0.8, 0.5, 1.).into());
        let orange = Mixed(gpui_kit::hsla(20. / 360., 0.8, 0.5, 1.).into());
        for step in 1..10 {
            let hue = Hsla::from(red.lerp(&orange, step as f32 / 10.).0).h * 360.;
            assert!(
                !(30.0..=340.0).contains(&hue),
                "step {step} passed through hue {hue}"
            );
        }
    }

    #[test]
    fn a_zero_duration_is_already_there() {
        assert_eq!(progress(MS(0), Duration::ZERO), 1.);
    }

    #[test]
    fn either_setting_snaps_and_the_layout_scales() {
        assert_eq!(scaled(FAST, true, 1.), None);
        assert_eq!(scaled(FAST, false, 0.), None);
        assert_eq!(scaled(FAST, true, 0.4), None);
        let ms = |scale| scaled(MEDIUM, false, scale).map(|span| span.as_secs_f32() * 1000.);
        assert!(ms(1.).is_some_and(|ms| (ms - 200.).abs() < 0.01));
        assert!(ms(0.4).is_some_and(|ms| (ms - 80.).abs() < 0.01));
    }

    #[test]
    fn a_fade_in_runs_from_hidden_to_shown() {
        let start = Instant::now();
        let mut fade = Fade::hidden();
        assert_eq!(fade.value_at(start), 0.);
        fade.turn_at(true, start, FAST);
        assert_eq!(fade.value_at(start), 0.);
        assert!(!fade.settled_at(start + MS(100)));
        let mid = fade.value_at(start + MS(75));
        assert!(mid > 0. && mid < 1.);
        assert_eq!(fade.value_at(start + FAST), 1.);
        assert!(fade.settled_at(start + FAST));
    }

    #[test]
    fn turning_back_mid_way_starts_from_where_it_stood() {
        let start = Instant::now();
        let mut fade = Fade::hidden();
        fade.turn_at(true, start, SLOW);
        let at = start + MS(100);
        let stood = fade.value_at(at);
        fade.turn_at(false, at, FAST);
        assert_eq!(fade.value_at(at), stood, "no jump on the turn");
        assert_eq!(fade.value_at(at + FAST), 0.);
        assert!(!fade.is_shown());
    }

    #[test]
    fn asking_for_the_same_side_again_does_not_restart_it() {
        let start = Instant::now();
        let mut fade = Fade::hidden();
        fade.turn_at(true, start, FAST);
        fade.turn_at(true, start + MS(100), FAST);
        assert_eq!(fade.value_at(start + FAST), 1.);
    }

    #[test]
    fn a_snap_lands_at_once() {
        let start = Instant::now();
        let mut fade = Fade::hidden();
        fade.turn_at(true, start, SLOW);
        fade.snap(false);
        assert_eq!(fade.value_at(start), 0.);
        assert!(fade.settled_at(start));
        fade.snap(true);
        assert_eq!(fade.value_at(start), 1.);
        assert!(fade.is_visible());
    }

    #[test]
    fn a_settled_hidden_fade_is_off_screen() {
        assert!(!Fade::hidden().is_visible());
        assert!(Fade::shown().is_visible());
    }
}
