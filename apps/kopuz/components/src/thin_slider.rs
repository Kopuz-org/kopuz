//! A hair-thin slider in a hit area you can actually hit: the paint the seek bar and the level share.

use gpui_kit::base::slider::{
    Slider as BareSlider, SliderEvent, SliderIndicator, SliderState, SliderTrack, SliderValue,
};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, App, Axis, Entity, HitboxBehavior, Hsla, IntoElement,
    ParentElement as _, Pixels, RenderOnce, SharedString, Styled as _, TouchDragEvent, TouchPhase,
    Window, canvas, div, px, relative,
};

use crate::{motion, skin, style};

const KNOB_CHANNEL: u64 = 90;
const SCAN_PERIOD: std::time::Duration = std::time::Duration::from_millis(1_400);
/// How much of the track the sweep covers; the rest is how far it has to travel.
const SCAN_WIDTH: f32 = 0.25;

// The Material 3 slider: a 16dp track, a 4x44dp bar handle with a 6dp gap either side, a 4dp stop at the end.
const M3_TRACK: Pixels = px(16.);
const M3_HANDLE: (Pixels, Pixels) = (px(4.), px(44.));
const M3_GAP: Pixels = px(6.);
const M3_STOP: Pixels = px(4.);
// The inner corner where the track meets the handle's gap.
const M3_INNER: Pixels = px(2.);
// The inactive track is the fill's tonal container: the same hue, far quieter.
const M3_CONTAINER: f32 = 0.24;
const M3_DISABLED: f32 = 0.38;

/// A slider's track, fill and knob over a `SliderState` the caller owns.
#[derive(IntoElement)]
pub struct ThinSlider {
    id: SharedString,
    state: Entity<SliderState>,
    filled: f32,
    hit: Pixels,
    bar: Pixels,
    knob: Pixels,
    show_knob: bool,
    disabled: bool,
    buffered: Vec<(f32, f32)>,
    scanning: bool,
    fill: Option<Hsla>,
}

impl ThinSlider {
    pub fn new(id: impl Into<SharedString>, state: &Entity<SliderState>) -> Self {
        Self {
            id: id.into(),
            state: state.clone(),
            filled: 0.,
            hit: px(16.),
            bar: px(3.),
            knob: px(10.),
            show_knob: false,
            disabled: false,
            buffered: Vec::new(),
            scanning: false,
            fill: None,
        }
    }

    /// How much of the track is filled, as a 0..1 fraction.
    pub fn filled(mut self, filled: f32) -> Self {
        self.filled = filled.clamp(0., 1.);
        self
    }

    /// The hit area's height, the paint's, and the knob's diameter when it is out.
    pub fn metrics(mut self, hit: Pixels, bar: Pixels, knob: Pixels) -> Self {
        self.hit = hit;
        self.bar = bar;
        self.knob = knob;
        self
    }

    /// Whether the knob is out, which it grows to and shrinks from rather than snapping.
    pub fn knob(mut self, shown: bool) -> Self {
        self.show_knob = shown;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Ranges already fetched, as `(left, right)` percentages of the track.
    pub fn buffered(mut self, ranges: Vec<(f32, f32)>) -> Self {
        self.buffered = ranges;
        self
    }

    /// A sweep across the track, saying a fetch is live rather than stalled.
    pub fn scanning(mut self, scanning: bool) -> Self {
        self.scanning = scanning;
        self
    }

    /// The fill and knob colour; the chrome's accent unless set.
    pub fn fill(mut self, fill: Hsla) -> Self {
        self.fill = Some(fill);
        self
    }
}

impl ThinSlider {
    /// How tall the slider stands for a hit area of `hit`: Material's bar handle is taller than any.
    pub fn height(hit: Pixels) -> Pixels {
        match skin::material() {
            true => hit.max(M3_HANDLE.1),
            false => hit,
        }
    }

    /// The Material 3 form: fill, gap, bar handle, gap, then the tonal rest with its stop dot.
    fn material(self, fill: Hsla) -> gpui_kit::AnyElement {
        let rest = fill.alpha(fill.a * M3_CONTAINER);
        let (handle_width, handle_height) = M3_HANDLE;
        let notch = M3_GAP * 2. + handle_width;
        let disabled = self.disabled;
        BareSlider::new(&self.state)
            .axis(Axis::Horizontal)
            .disabled(disabled)
            .w_full()
            .when(disabled, |slider| slider.opacity(M3_DISABLED))
            .child(
                SliderTrack::new(&self.state)
                    .axis(Axis::Horizontal)
                    .disabled(disabled)
                    .w_full()
                    .child(
                        // Only the indicator records the bounds a press maps against.
                        SliderIndicator::new(&self.state)
                            .w_full()
                            .h(handle_height)
                            .flex()
                            .items_center()
                            .child(
                                div()
                                    .h(M3_TRACK)
                                    .w(relative(self.filled))
                                    .flex_shrink(1.)
                                    .rounded_l(M3_TRACK / 2.)
                                    .rounded_r(M3_INNER)
                                    .bg(fill),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .w(notch)
                                    .h(handle_height)
                                    .flex()
                                    .justify_center()
                                    .child(
                                        div()
                                            .w(handle_width)
                                            .h_full()
                                            .rounded(handle_width / 2.)
                                            .bg(fill),
                                    ),
                            )
                            .child(
                                div()
                                    .relative()
                                    .flex_1()
                                    .min_w_0()
                                    .h(M3_TRACK)
                                    .rounded_l(M3_INNER)
                                    .rounded_r(M3_TRACK / 2.)
                                    .bg(rest)
                                    .child(
                                        div()
                                            .absolute()
                                            .top((M3_TRACK - M3_STOP) / 2.)
                                            .right(M3_GAP)
                                            .size(M3_STOP)
                                            .rounded_full()
                                            .bg(fill),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }
}

impl RenderOnce for ThinSlider {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        if !crate::phone::is_touch(cx) {
            return self.render_slider(window, cx);
        }
        let touch = touch_input(&self.id, self.state.clone(), self.disabled, window, cx);
        div()
            .relative()
            .w_full()
            .min_h(crate::phone::TOUCH_TARGET)
            .flex()
            .items_center()
            .child(self.render_slider(window, cx))
            .child(touch)
            .into_any_element()
    }
}

impl ThinSlider {
    fn render_slider(self, window: &mut Window, cx: &mut App) -> AnyElement {
        let fill = self.fill.unwrap_or_else(|| {
            motion::accent(theme::Surface::Chrome, cx.theme().foreground, window, cx)
        });
        if skin::material() {
            return self.material(fill);
        }
        let track = style::white(style::ACTIVE, cx);
        let spans = style::white(style::TEXT_DIM, cx);
        let knob = motion::length(
            motion::channel(&self.id, KNOB_CHANNEL),
            if self.show_knob && !self.disabled {
                self.knob
            } else {
                px(0.)
            },
            motion::FAST,
            window,
            cx,
        );
        let (bar, filled) = (self.bar, self.filled);

        BareSlider::new(&self.state)
            .axis(Axis::Horizontal)
            .disabled(self.disabled)
            .w_full()
            .child(
                SliderTrack::new(&self.state)
                    .axis(Axis::Horizontal)
                    .disabled(self.disabled)
                    .w_full()
                    .h(self.hit)
                    .flex()
                    .items_center()
                    .child(
                        SliderIndicator::new(&self.state)
                            .relative()
                            .w_full()
                            .h(bar)
                            .rounded_full()
                            .bg(track)
                            .children(
                                self.buffered
                                    .into_iter()
                                    .map(|(left, right)| span(left, right, spans)),
                            )
                            .when(self.scanning, |bar| bar.child(scan(fill)))
                            .child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .left_0()
                                    .h_full()
                                    .w(relative(filled))
                                    .rounded_full()
                                    .bg(fill),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .top(-(knob - bar) / 2.)
                                    .left(relative(filled))
                                    .ml(-knob / 2.)
                                    .size(knob)
                                    .rounded_full()
                                    .bg(fill),
                            ),
                    ),
            )
            .into_any_element()
    }
}

/// Claims the finger before GPUI turns its movement into scrolling; state survives redraws mid-drag.
fn touch_input(
    id: &SharedString,
    state: Entity<SliderState>,
    disabled: bool,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let held = window.use_keyed_state(motion::channel(id, 91), cx, |_, _| None::<SliderValue>);
    canvas(
        |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
        move |_, hitbox, window, _| {
            window.on_mouse_event(move |event: &TouchDragEvent, phase, window, cx| {
                if !phase.bubble() || disabled {
                    return;
                }
                if event.phase == TouchPhase::Started {
                    if window.default_prevented() || !hitbox.is_hovered(window) {
                        return;
                    }
                    let value = state.read(cx).value();
                    held.update(cx, |held, _| *held = Some(value));
                    window.prevent_default();
                }
                let Some(original) = *held.read(cx) else {
                    return;
                };
                cx.stop_propagation();
                state.update(cx, |state, cx| {
                    if event.phase == TouchPhase::Cancelled {
                        state.set_value(original, window, cx);
                        cx.emit(SliderEvent::Change(original));
                    } else if state.bounds().size.width > px(0.) {
                        state.update_value_by_position(
                            Axis::Horizontal,
                            event.position,
                            false,
                            window,
                            cx,
                        );
                    }
                    if matches!(event.phase, TouchPhase::Ended | TouchPhase::Cancelled) {
                        state.handle_release(cx);
                    }
                });
                if matches!(event.phase, TouchPhase::Ended | TouchPhase::Cancelled) {
                    held.update(cx, |held, _| *held = None);
                }
            });
        },
    )
    .absolute()
    .inset_0()
    .into_any_element()
}

/// One buffered range, as a fraction of the track it covers.
fn span(left: f32, right: f32, color: Hsla) -> impl IntoElement {
    div()
        .absolute()
        .top_0()
        .h_full()
        .left(relative(left / 100.))
        .w(relative((right - left).max(0.) / 100.))
        .rounded_full()
        .bg(color)
}

fn scan(color: Hsla) -> impl IntoElement {
    div()
        .absolute()
        .top_0()
        .h_full()
        .rounded_full()
        .bg(color)
        .with_animation(
            "seek-scan",
            Animation::new(SCAN_PERIOD).repeat(),
            |bar, progress| {
                // Both edges stay inside the track, since an overflow clip is a rectangle here.
                let lead = progress * (1. + SCAN_WIDTH);
                let (left, right) = ((lead - SCAN_WIDTH).max(0.), lead.min(1.));
                bar.left(relative(left)).w(relative(right - left))
            },
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{AppContext as _, Context, Render, TestAppContext, point};

    struct Fixture {
        state: Entity<SliderState>,
    }

    impl Render for Fixture {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .w(px(200.))
                .child(ThinSlider::new("probe", &self.state))
        }
    }

    #[gpui_kit::test]
    fn a_press_on_a_material_slider_lands_where_it_was_pressed(cx: &mut TestAppContext) {
        skin::register(Box::new(skin::Material3));
        skin::select("material3");
        assert!(skin::material());
        // The link talks to its tokio runtime from another thread, which the test scheduler refuses by default.
        cx.executor().allow_parking();
        cx.update(|cx| {
            gpui_kit::init(cx);
            i18n::init("en");
            link::init(
                link::Address::Socket("/nonexistent/kopuz.sock".into()),
                false,
                cx,
            )
            .expect("a link that has not connected yet");
            store::init(cx);
            theme::apply_named("default", None, cx);
        });
        let state = cx.new(|_| SliderState::new().min(0.).max(1.).step(0.001));
        let (_, cx) = cx.add_window_view(|_, _| Fixture {
            state: state.clone(),
        });
        cx.run_until_parked();
        cx.simulate_click(point(px(150.), px(22.)), Default::default());
        let value = state.read_with(cx, |state, _| state.value().end());
        assert!((value - 0.75).abs() < 0.01, "pressed at 3/4, got {value}");
    }

    struct TouchFixture {
        state: Entity<SliderState>,
        disabled: bool,
        _events: gpui_kit::Subscription,
    }

    impl Render for TouchFixture {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().w(px(200.)).child(
                ThinSlider::new("touch-probe", &self.state)
                    .fill(gpui_kit::Hsla::default())
                    .disabled(self.disabled),
            )
        }
    }

    #[gpui_kit::test]
    fn touch_slider_drags_release_once_and_cancel_restores_value(cx: &mut TestAppContext) {
        use gpui_kit::{TouchEvent, TouchId};
        use std::{cell::RefCell, rc::Rc};

        cx.executor().allow_parking();
        cx.update(|cx| {
            gpui_kit::init(cx);
            i18n::init("en");
            link::init(
                link::Address::Socket("/nonexistent/kopuz.sock".into()),
                false,
                cx,
            )
            .expect("an unconnected link");
            store::init(cx);
            theme::apply_named("default", None, cx);
            theme::mobile::set_mobile(Default::default(), cx);
        });
        let state = cx.new(|_| SliderState::new().min(0.).max(1.).step(0.001));
        let releases = Rc::new(RefCell::new(Vec::new()));
        let (view, cx) = cx.add_window_view(|_, cx| TouchFixture {
            state: state.clone(),
            disabled: false,
            _events: cx.subscribe(&state, {
                let releases = releases.clone();
                move |_, _, event, _| {
                    if let SliderEvent::Release(value) = event {
                        releases.borrow_mut().push(value.end());
                    }
                }
            }),
        });
        cx.run_until_parked();
        let touch = |id, phase, x| TouchEvent {
            id: TouchId(id),
            phase,
            position: point(px(x), px(24.)),
            ..Default::default()
        };
        cx.simulate_event(touch(1, TouchPhase::Started, 50.));
        cx.simulate_event(touch(1, TouchPhase::Moved, 150.));
        cx.run_until_parked();
        assert!((state.read_with(cx, |state, _| state.value().end()) - 0.75).abs() < 0.01);
        cx.simulate_event(touch(1, TouchPhase::Ended, 180.));
        cx.run_until_parked();
        assert_eq!(releases.borrow().len(), 1);
        assert!((releases.borrow()[0] - 0.9).abs() < 0.01);

        cx.simulate_event(touch(2, TouchPhase::Started, 20.));
        cx.simulate_event(touch(2, TouchPhase::Moved, 50.));
        cx.simulate_event(touch(2, TouchPhase::Cancelled, 50.));
        cx.run_until_parked();
        assert!((state.read_with(cx, |state, _| state.value().end()) - 0.9).abs() < 0.01);
        assert_eq!(releases.borrow().len(), 2);

        view.update(cx, |view, cx| {
            view.disabled = true;
            cx.notify();
        });
        cx.run_until_parked();
        cx.simulate_event(touch(3, TouchPhase::Started, 40.));
        cx.simulate_event(touch(3, TouchPhase::Moved, 100.));
        cx.simulate_event(touch(3, TouchPhase::Ended, 100.));
        cx.run_until_parked();
        assert_eq!(releases.borrow().len(), 2);
    }
}
