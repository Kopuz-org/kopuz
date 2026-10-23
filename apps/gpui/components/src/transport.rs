//! Shuffle, previous, play, next and repeat, each keying its own hover.

use std::rc::Rc;

use api::LoopMode;
use gpui_kit::component::button::Button;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::component::{ActiveTheme as _, Disableable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, ClickEvent, FontWeight, Hsla, IntoElement, ParentElement as _, Pixels, RenderOnce,
    SharedString, Styled as _, Window, div, px, relative,
};

use crate::cover_overlay::PlayState;
use crate::format::t;
use crate::icon_button::{IconButton, IconTone};
use crate::menu_button::{MenuButton, MenuVariant};
use crate::menus::Menu;
use crate::{icons, motion, style};

/// The channel the play button keeps its glyph swap under.
const GLYPH_SWAP: u64 = 30;

// Lucide names the shared icon set has no constant for yet.
const TIMER: &str = "icons/timer.svg";
const GAUGE: &str = "icons/gauge.svg";

// `text-[9px] leading-none`: the "1" over a repeat-one button.
const BADGE_TEXT: Pixels = px(9.);

type Press = Rc<dyn Fn(&mut Window, &mut App)>;
type Click = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// How big one surface draws the transport.
#[derive(Debug, Clone, Copy)]
pub struct TransportMetrics {
    pub side: Pixels,
    pub step: Pixels,
    pub play: Pixels,
    pub side_glyph: Pixels,
    pub step_glyph: Pixels,
    pub play_glyph: Pixels,
    pub gap: Pixels,
}

impl TransportMetrics {
    /// The bar's, and the mini window's: small enough to sit under a 96px bar.
    pub const COMPACT: Self = Self {
        side: px(36.),
        step: px(40.),
        play: px(40.),
        side_glyph: px(14.),
        step_glyph: px(20.),
        // The Dioxus bar drew this smaller and let a solid glyph carry the weight; outlines need the size.
        play_glyph: px(22.),
        gap: px(8.),
    };

    /// The sheet's: the one surface with room for a real transport.
    pub const HERO: Self = Self {
        side: px(44.),
        step: px(56.),
        play: px(64.),
        side_glyph: px(18.),
        step_glyph: px(26.),
        play_glyph: px(28.),
        gap: px(16.),
    };

    /// Every measure multiplied by `scale`, for a surface the person has made larger or smaller.
    pub fn scaled(self, scale: f32) -> Self {
        Self {
            side: self.side * scale,
            step: self.step * scale,
            play: self.play * scale,
            side_glyph: self.side_glyph * scale,
            step_glyph: self.step_glyph * scale,
            play_glyph: self.play_glyph * scale,
            gap: self.gap * scale,
        }
    }
}

/// Play or pause: a round glyph in a transport, or a labelled pill on a card.
#[derive(IntoElement)]
pub struct PlayPauseButton {
    id: SharedString,
    playing: bool,
    size: Pixels,
    glyph: Pixels,
    pill: bool,
    disabled: bool,
    stop: bool,
    click: Option<Click>,
    loading: bool,
    floating: bool,
}

impl PlayPauseButton {
    pub fn new(id: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            playing: false,
            size: px(40.),
            glyph: px(22.),
            pill: false,
            disabled: false,
            stop: false,
            click: None,
            loading: false,
            floating: false,
        }
    }

    pub fn playing(mut self, playing: bool) -> Self {
        self.playing = playing;
        self
    }

    pub fn size(mut self, size: Pixels, glyph: Pixels) -> Self {
        self.size = size;
        self.glyph = glyph;
        self
    }

    /// Draw it as the inverted pill with its word beside the glyph.
    pub fn pill(mut self, pill: bool) -> Self {
        self.pill = pill;
        self
    }

    /// The accent button over a cover's corner, which shows `state` and is no click on the card under it.
    pub fn cover(mut self, state: PlayState) -> Self {
        self.playing = state == PlayState::Playing;
        self.loading = state == PlayState::Loading;
        self.floating = true;
        self.size = px(40.);
        self.glyph = px(14.);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Keep the press from reaching what the button sits on, such as a card that would open.
    pub fn stop_propagation(mut self, stop: bool) -> Self {
        self.stop = stop;
        self
    }

    pub fn on_click(
        mut self,
        click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.click = Some(Rc::new(click));
        self
    }
}

/// Which glyph a play button last drew, and its crossfade to the one it draws now.
struct GlyphSwap {
    playing: bool,
    fade: motion::Fade,
}

/// How far the play/pause glyph is through a swap: `Some(shown)` while it crossfades.
fn glyph_swap(id: &SharedString, playing: bool, window: &mut Window, cx: &mut App) -> Option<f32> {
    let state = window.use_keyed_state(motion::channel(id, GLYPH_SWAP), cx, |_, _| GlyphSwap {
        playing,
        fade: motion::Fade::shown(),
    });
    let (swap, shown) = state.update(cx, |swap, cx| {
        if swap.playing != playing {
            swap.playing = playing;
            swap.fade = motion::Fade::hidden();
        }
        let shown = swap.fade.sample(true, motion::GLYPH, window, cx);
        (shown < 1., shown)
    });
    swap.then_some(shown)
}

impl RenderOnce for PlayPauseButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let glyph = match self.playing {
            true => icons::PAUSE,
            false => icons::PLAY,
        };
        let was = match self.playing {
            true => icons::PLAY,
            false => icons::PAUSE,
        };
        let swap = glyph_swap(&self.id, self.playing, window, cx);
        let word = t(if self.playing { "pause" } else { "play" });
        let click = self.click;
        if self.pill {
            let stop = self.stop;
            return style::inverted(Button::new(self.id), cx)
                .h_11()
                .px_6()
                .rounded_full()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .icon(icons::icon(glyph))
                .label(word)
                .disabled(self.disabled)
                .when_some(click, |button, click| {
                    button.on_click(move |event, window, cx| {
                        if stop {
                            cx.stop_propagation();
                        }
                        click(event, window, cx);
                    })
                })
                .into_any_element();
        }
        if self.floating {
            return IconButton::new(self.id, glyph)
                .when_some(swap, |button, shown| button.swap_from(was, shown))
                .size(self.size)
                .glyph(self.glyph)
                .tone(IconTone::Accent)
                .shadow(true)
                .loading(self.loading)
                .stop_propagation(true)
                .when_some(click, |button, click| {
                    button.on_click(move |event, window, cx| click(event, window, cx))
                })
                .into_any_element();
        }
        // A skin that squares the play button fills it, as Material's filled icon button is.
        let shape = style::chrome().play_shape;
        IconButton::new(self.id, glyph)
            .when_some(swap, |button, shown| button.swap_from(was, shown))
            .size(self.size)
            .glyph(self.glyph)
            .tone(match shape {
                Some(_) => IconTone::Accent,
                None => IconTone::Quiet,
            })
            .shape(shape)
            .lit(shape.is_none())
            .when(shape.is_none(), |button| {
                button.accent(cx.theme().foreground)
            })
            .disabled(self.disabled)
            .stop_propagation(self.stop)
            .tooltip(word)
            .when_some(click, |button, click| {
                button.on_click(move |event, window, cx| click(event, window, cx))
            })
            .into_any_element()
    }
}

/// Repeat, lit once it is on, with a "1" over it when it repeats the one track.
#[derive(IntoElement)]
pub struct RepeatButton {
    id: SharedString,
    mode: LoopMode,
    size: Pixels,
    glyph: Pixels,
    click: Option<Press>,
}

impl RepeatButton {
    pub fn new(id: impl Into<SharedString>, mode: LoopMode) -> Self {
        Self {
            id: id.into(),
            mode,
            size: px(36.),
            glyph: px(14.),
            click: None,
        }
    }

    pub fn size(mut self, size: Pixels, glyph: Pixels) -> Self {
        self.size = size;
        self.glyph = glyph;
        self
    }

    pub fn on_click(mut self, click: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.click = Some(Rc::new(click));
        self
    }
}

impl RenderOnce for RepeatButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let accent = motion::accent(theme::Surface::Chrome, cx.theme().primary, window, cx);
        let repeating = self.mode != LoopMode::None;
        let tip = match self.mode {
            LoopMode::None => t("repeat_off"),
            LoopMode::Queue => t("repeat_queue"),
            LoopMode::Track => t("repeat_track"),
        };
        let click = self.click;
        div()
            .relative()
            .flex()
            .flex_shrink_0()
            .child(
                IconButton::new(self.id, icons::REPEAT)
                    .size(self.size)
                    .glyph(self.glyph)
                    .lit(repeating)
                    .accent(accent)
                    .tooltip(tip)
                    .when_some(click, |button, click| {
                        button.on_click(move |_, window, cx| click(window, cx))
                    }),
            )
            .when(self.mode == LoopMode::Track, |wrapper| {
                wrapper.child(
                    div()
                        .absolute()
                        .bottom(px(2.))
                        .left_0()
                        .right_0()
                        .text_center()
                        .text_size(BADGE_TEXT)
                        .line_height(relative(1.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(accent)
                        .child("1"),
                )
            })
    }
}

/// Shuffle, previous, play, next and repeat; the side toggles are the preferences' to draw.
#[derive(IntoElement)]
pub struct Transport {
    id: SharedString,
    metrics: TransportMetrics,
    sides: bool,
    balanced: bool,
    playing: bool,
    loaded: bool,
    shuffle: Option<bool>,
    repeat: Option<LoopMode>,
    toggle: Option<Press>,
    previous: Option<Press>,
    next: Option<Press>,
    on_shuffle: Option<Press>,
    on_repeat: Option<Press>,
}

impl Transport {
    pub fn new(id: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            metrics: TransportMetrics::COMPACT,
            sides: false,
            balanced: false,
            playing: false,
            loaded: false,
            shuffle: None,
            repeat: None,
            toggle: None,
            previous: None,
            next: None,
            on_shuffle: None,
            on_repeat: None,
        }
    }

    pub fn metrics(mut self, metrics: TransportMetrics) -> Self {
        self.metrics = metrics;
        self
    }

    /// Draw the optional side toggles at all; off leaves previous, play and next alone.
    pub fn sides(mut self, sides: bool) -> Self {
        self.sides = sides;
        self
    }

    /// Hold a blank where a side toggle is left out, so play stays centred under a column.
    pub fn balanced(mut self, balanced: bool) -> Self {
        self.balanced = balanced;
        self
    }

    pub fn playing(mut self, playing: bool) -> Self {
        self.playing = playing;
        self
    }

    /// Whether a track is loaded; without one the three main buttons are disabled.
    pub fn loaded(mut self, loaded: bool) -> Self {
        self.loaded = loaded;
        self
    }

    /// The shuffle toggle, with whether it is on; `None` is a preference that leaves it out.
    pub fn shuffle(mut self, shuffle: Option<bool>) -> Self {
        self.shuffle = shuffle;
        self
    }

    /// The repeat toggle, with its mode; `None` is a preference that leaves it out.
    pub fn repeat(mut self, repeat: Option<LoopMode>) -> Self {
        self.repeat = repeat;
        self
    }

    pub fn on_toggle(mut self, press: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.toggle = Some(Rc::new(press));
        self
    }

    pub fn on_previous(mut self, press: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.previous = Some(Rc::new(press));
        self
    }

    pub fn on_next(mut self, press: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.next = Some(Rc::new(press));
        self
    }

    pub fn on_shuffle(mut self, press: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_shuffle = Some(Rc::new(press));
        self
    }

    pub fn on_repeat(mut self, press: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_repeat = Some(Rc::new(press));
        self
    }
}

fn press(press: Option<Press>) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    move |_, window, cx| {
        if let Some(press) = &press {
            press(window, cx);
        }
    }
}

impl RenderOnce for Transport {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let metrics = self.metrics;
        let id = self.id;
        let accent = motion::accent(theme::Surface::Chrome, cx.theme().primary, window, cx);
        let key = |name: &str| SharedString::from(format!("{id}-{name}"));
        let shuffle = self.shuffle.filter(|_| self.sides);
        let repeat = self.repeat.filter(|_| self.sides);
        let step = |name: &str, glyph: &'static str, tip: &'static str, handler: Option<Press>| {
            IconButton::new(key(name), glyph)
                .size(metrics.step)
                .glyph(metrics.step_glyph)
                .disabled(!self.loaded)
                .tooltip(t(tip))
                .on_click(press(handler))
        };

        gpui_kit::component::h_flex()
            .gap(metrics.gap)
            .items_center()
            .when(
                self.balanced && shuffle.is_none() && repeat.is_some(),
                |row| row.child(div().w(metrics.side).flex_shrink_0()),
            )
            .children(shuffle.map(|on| {
                IconButton::new(key("shuffle"), icons::SHUFFLE)
                    .size(metrics.side)
                    .glyph(metrics.side_glyph)
                    .lit(on)
                    .accent(accent)
                    .tooltip(t(if on { "shuffle_on" } else { "shuffle_off" }))
                    .on_click(press(self.on_shuffle))
            }))
            .child(step(
                "previous",
                icons::SKIP_BACK,
                "previous_track",
                self.previous,
            ))
            .child(
                PlayPauseButton::new(key("play-pause"))
                    .playing(self.playing)
                    .size(metrics.play, metrics.play_glyph)
                    .disabled(!self.loaded)
                    .on_click(press(self.toggle)),
            )
            .child(step("next", icons::SKIP_FORWARD, "next_track", self.next))
            .children(repeat.map(|mode| {
                let on_repeat = self.on_repeat;
                RepeatButton::new(key("repeat"), mode)
                    .size(metrics.side, metrics.side_glyph)
                    .on_click(move |window, cx| {
                        if let Some(on_repeat) = &on_repeat {
                            on_repeat(window, cx);
                        }
                    })
            }))
            .when(
                self.balanced && repeat.is_none() && shuffle.is_some(),
                |row| row.child(div().w(metrics.side).flex_shrink_0()),
            )
    }
}

/// The overflow beside the transport: where a sleep timer and a playback speed will live.
#[derive(IntoElement)]
pub struct TransportOverflow {
    id: SharedString,
    size: Pixels,
    glyph: Pixels,
}

impl TransportOverflow {
    pub fn new(id: impl Into<SharedString>, size: Pixels, glyph: Pixels) -> Self {
        Self {
            id: id.into(),
            size,
            glyph,
        }
    }
}

impl RenderOnce for TransportOverflow {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let menu: Menu = Rc::new(|menu, _, _| {
            menu.label(t("playback_extras"))
                .item(
                    PopupMenuItem::new(t("sleep_timer"))
                        .icon(icons::icon(TIMER))
                        .disabled(true),
                )
                .item(
                    PopupMenuItem::new(t("playback_speed"))
                        .icon(icons::icon(GAUGE))
                        .disabled(true),
                )
        });
        MenuButton::new(self.id, menu).variant(MenuVariant::Icon {
            size: self.size,
            glyph: self.glyph,
            tone: IconTone::default(),
        })
    }
}

/// The colour a lit control wears, for a surface drawing its own beside the transport.
pub fn lit_accent(window: &mut Window, cx: &mut App) -> Hsla {
    motion::accent(theme::Surface::Chrome, cx.theme().primary, window, cx)
}
