//! The volume control: the slider's position is the level sent, and the player engine applies the loudness curve.

use std::rc::Rc;

use gpui_kit::base::slider::SliderState;
use gpui_kit::component::h_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Pixels, RenderOnce,
    ScrollDelta, ScrollWheelEvent, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, div, px, rems,
};

use crate::format::t;
use crate::icon_button::IconButton;
use crate::thin_slider::ThinSlider;
use crate::{icons, motion, style};

const HOVER: u64 = 1;
const WHEEL: u64 = 2;

// `w-24`, the width the bar gives the level.
const TRACK_WIDTH: Pixels = px(96.);
// A 3px paint in a hit area tall enough to catch a pointer aiming at it.
const TRACK_HIT: Pixels = px(16.);
const TRACK_BAR: Pixels = px(3.);
const KNOB: Pixels = px(10.);
const GLYPH: Pixels = px(14.);
// A wheel notch this many pixels of travel; a trackpad flick used to arrive as one step per event.
const PIXELS_PER_NOTCH: f32 = 60.;
// However hard the flick, one event never moves the level more than this many notches.
const MAX_NOTCHES: f32 = 3.;
const SILENT: f32 = f32::EPSILON;

fn glyph_for(level: f32, muted: bool) -> &'static str {
    if muted || level <= SILENT {
        icons::VOLUME_MUTED
    } else if level < 0.5 {
        icons::VOLUME_LOW
    } else {
        icons::VOLUME
    }
}

type Mute = Rc<dyn Fn(&mut Window, &mut App)>;
type Step = Rc<dyn Fn(f32, &mut Window, &mut App)>;

/// The mute glyph and the level beside it, sharing one wheel.
#[derive(IntoElement)]
pub struct VolumeControl {
    id: SharedString,
    state: Entity<SliderState>,
    level: f32,
    muted: bool,
    slider: bool,
    wide: bool,
    held: bool,
    scroll_step: f32,
    scale: f32,
    mute: Option<Mute>,
    step: Option<Step>,
}

impl VolumeControl {
    pub fn new(id: impl Into<SharedString>, state: &Entity<SliderState>) -> Self {
        Self {
            id: id.into(),
            state: state.clone(),
            level: 1.,
            muted: false,
            slider: true,
            wide: false,
            held: false,
            scroll_step: 0.,
            scale: 1.,
            mute: None,
            step: None,
        }
    }

    /// The wire level, which picks the glyph.
    pub fn level(mut self, level: f32) -> Self {
        self.level = level;
        self
    }

    /// Output is silenced while the level is kept, which picks the muted glyph.
    pub fn muted(mut self, muted: bool) -> Self {
        self.muted = muted;
        self
    }

    /// Draw the track at all; without it the glyph still takes the wheel.
    pub fn slider(mut self, slider: bool) -> Self {
        self.slider = slider;
        self
    }

    /// Fill the width it is given, as the sheet does, rather than the bar's fixed track.
    pub fn wide(mut self, wide: bool) -> Self {
        self.wide = wide;
        self
    }

    /// The pointer is holding the track, which keeps the knob out.
    pub fn held(mut self, held: bool) -> Self {
        self.held = held;
        self
    }

    /// How far one wheel notch moves the slider; zero turns the wheel off.
    pub fn scroll_step(mut self, step: f32) -> Self {
        self.scroll_step = step;
        self
    }

    /// Every measure multiplied by `scale`, for a bar the person has made larger or smaller.
    pub fn scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }

    pub fn on_mute(mut self, mute: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.mute = Some(Rc::new(mute));
        self
    }

    /// The slider reading the wheel moved to, for the caller to show and send.
    pub fn on_step(mut self, step: impl Fn(f32, &mut Window, &mut App) + 'static) -> Self {
        self.step = Some(Rc::new(step));
        self
    }
}

impl RenderOnce for VolumeControl {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (over, pointer) = motion::hovered(motion::channel(&self.id, HOVER), window, cx);
        let wheel = window.use_keyed_state(motion::channel(&self.id, WHEEL), cx, |_, _| 0f32);
        let filled = self.state.read(cx).percentage().end;
        let state = self.state.clone();
        let (step, notch, mute) = (self.step, self.scroll_step.max(0.), self.mute);
        let wide = self.wide;
        let scale = self.scale;
        let button = style::chrome().round_button * scale;

        h_flex()
            .id(self.id.clone())
            .when(wide, |control| control.w_full().max_w(rems(18.)))
            .gap_2()
            .items_center()
            .on_hover(motion::track(pointer))
            .on_scroll_wheel(move |event: &ScrollWheelEvent, window, cx| {
                let Some(step) = step.as_ref().filter(|_| notch > 0.) else {
                    return;
                };
                let notches = match event.delta {
                    ScrollDelta::Pixels(delta) => f32::from(delta.y) / PIXELS_PER_NOTCH,
                    ScrollDelta::Lines(delta) => delta.y,
                };
                // Plain `update` rather than a notify: a half notch changes nothing that is drawn.
                let whole = wheel.update(cx, |wheel, _| {
                    *wheel = (*wheel + notches).clamp(-MAX_NOTCHES, MAX_NOTCHES);
                    let whole = wheel.trunc();
                    *wheel -= whole;
                    whole
                });
                if whole == 0. {
                    return;
                }
                let shown = state.read(cx).value().end();
                step((shown + whole * notch).clamp(0., 1.), window, cx);
            })
            .child(
                IconButton::new(
                    SharedString::from(format!("{}-mute", self.id)),
                    glyph_for(self.level, self.muted),
                )
                .size(button)
                .glyph(GLYPH * scale)
                .tooltip(t("toggle_mute"))
                .on_click(move |_, window, cx| {
                    if let Some(mute) = &mute {
                        mute(window, cx);
                    }
                }),
            )
            .when(self.slider, |control| {
                control.child(
                    div()
                        .when(wide, |track| track.flex_1().min_w_0())
                        .when(!wide, |track| track.w(TRACK_WIDTH * scale))
                        .child(
                            ThinSlider::new(self.id.clone(), &self.state)
                                .filled(filled)
                                .metrics(TRACK_HIT * scale, TRACK_BAR * scale, KNOB * scale)
                                .knob(over || self.held),
                        ),
                )
            })
            .when(wide, |control| {
                control.child(div().w(button).flex_shrink_0())
            })
    }
}
