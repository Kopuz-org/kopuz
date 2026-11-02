//! The one round glyph button: a hero's play, shuffle and download, a page's add, a cover's corner.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, Hsla, IntoElement, ParentElement as _, Pixels, RenderOnce,
    SharedString, Styled as _, Window, div, px,
};

use crate::{focus, hover, icons, motion, skin, style};

const HOVER: u64 = 1;
const WASH: u64 = 2;
const INK: u64 = 3;
const EDGE: u64 = 4;

/// `border-white/20 hover:border-white/40`, an outlined button's ring.
const OUTLINE: (f32, f32) = (0.20, 0.40);
/// What a disabled button fades to, since its frame paints the fill a Button would have dimmed.
const DISABLED: f32 = 0.5;
/// `border-1`: the ring's width, which the button inside it leaves room for.
const RING: Pixels = px(1.);

/// What a round button's colours say.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum IconTone {
    /// `bg-indigo-500 hover:bg-indigo-400`: the one filled action, such as play.
    Accent,
    /// No fill until the pointer washes it; slate glyph that whitens, or the accent while lit.
    #[default]
    Quiet,
    /// A ringed button whose white/70 glyph whitens, washing under the pointer.
    Outline,
    /// `bg-black/40 hover:bg-black/60` over a cover or a list, ringed in white/10.
    Scrim,
    /// `bg-white/10 hover:bg-white/15` with a white/70 glyph: the buttons beside a hero title.
    Soft,
}

impl IconTone {
    /// The fill, the glyph, and the ring, for a pointer over it or not.
    fn colours(
        self,
        hot: bool,
        lit: bool,
        accent: Option<Hsla>,
        cx: &App,
    ) -> (Hsla, Hsla, Option<(f32, f32)>) {
        let wash = style::white(if hot { style::HOVER } else { 0. }, cx);
        let ink = |rest: Hsla| match (lit, hot) {
            (true, _) => accent.unwrap_or(cx.theme().primary),
            (false, true) => cx.theme().foreground,
            (false, false) => rest,
        };
        match self {
            // A given accent fills it instead; the background reads on it as well as it reads on the background.
            IconTone::Accent if accent.is_some() => (
                accent
                    .map(|accent| if hot { accent.opacity(0.85) } else { accent })
                    .unwrap_or(cx.theme().primary),
                cx.theme().background,
                None,
            ),
            IconTone::Accent => (
                match hot {
                    true => cx.theme().primary_hover,
                    false => cx.theme().primary,
                },
                cx.theme().primary_foreground,
                None,
            ),
            IconTone::Quiet => (wash, ink(style::slate_400(cx)), None),
            IconTone::Outline => (wash, ink(style::white(0.7, cx)), Some(OUTLINE)),
            IconTone::Soft => (
                style::white(if hot { 0.15 } else { style::HOVER }, cx),
                ink(style::white(0.7, cx)),
                None,
            ),
            IconTone::Scrim => (
                style::black(if hot { 0.6 } else { 0.4 }, cx),
                ink(style::white(style::TEXT_STRONG, cx)),
                Some((style::BORDER, style::BORDER)),
            ),
        }
    }
}

type OnClick = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
type Host = Box<dyn FnOnce(Button) -> AnyElement>;

/// A circle holding one glyph; its frame owns the hover, so the colours tween and a tooltip still fits.
#[derive(IntoElement)]
pub struct IconButton {
    id: SharedString,
    icon: &'static str,
    size: Pixels,
    glyph: Option<Pixels>,
    tone: IconTone,
    ring: Option<(f32, f32)>,
    lit: bool,
    accent: Option<Hsla>,
    disabled: bool,
    loading: bool,
    stop: bool,
    shadow: bool,
    tooltip: Option<SharedString>,
    click: Option<OnClick>,
    host: Option<Host>,
    // Width and corner as shares of the height, for a button that is not a circle.
    shape: Option<skin::PlayShape>,
    swap: Option<(&'static str, f32)>,
}

impl IconButton {
    pub fn new(id: impl Into<SharedString>, icon: &'static str) -> Self {
        Self {
            id: id.into(),
            icon,
            size: px(40.),
            glyph: None,
            tone: IconTone::Quiet,
            ring: None,
            lit: false,
            accent: None,
            disabled: false,
            loading: false,
            stop: false,
            shadow: false,
            tooltip: None,
            click: None,
            host: None,
            shape: None,
            swap: None,
        }
    }

    /// A rounded rectangle `size` tall rather than a circle.
    pub fn shape(mut self, shape: Option<skin::PlayShape>) -> Self {
        self.shape = shape;
        self
    }

    /// The circle's diameter; the glyph follows at about a third of it unless set.
    pub fn size(mut self, size: Pixels) -> Self {
        self.size = size;
        self
    }

    pub fn glyph(mut self, glyph: Pixels) -> Self {
        self.glyph = Some(glyph);
        self
    }

    pub fn tone(mut self, tone: IconTone) -> Self {
        self.tone = tone;
        self
    }

    /// A ring at white/`rest`, brightening to white/`hot` under the pointer, on any tone.
    pub fn ring(mut self, rest: f32, hot: f32) -> Self {
        self.ring = Some((rest, hot));
        self
    }

    /// Already on, such as shuffle: the glyph takes the accent.
    pub fn lit(mut self, lit: bool) -> Self {
        self.lit = lit;
        self
    }

    /// The colour a lit glyph takes in place of the theme's primary, such as the cover-tinted accent.
    pub fn accent(mut self, accent: Hsla) -> Self {
        self.accent = Some(accent);
        self
    }

    /// The glyph it is changing from, `shown` of the way to its own, the two crossfading in place.
    pub fn swap_from(mut self, from: &'static str, shown: f32) -> Self {
        self.swap = (shown < 1.).then_some((from, shown));
        self
    }

    /// Wrap the inner button in something that needs it as a trigger, such as a popover.
    pub fn host(mut self, host: impl FnOnce(Button) -> AnyElement + 'static) -> Self {
        self.host = Some(Box::new(host));
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    /// Keep the press from reaching what the button sits on, such as a row that would play.
    pub fn stop_propagation(mut self, stop: bool) -> Self {
        self.stop = stop;
        self
    }

    /// `shadow-md`, for a button floating over a cover rather than sitting on the page.
    pub fn shadow(mut self, shadow: bool) -> Self {
        self.shadow = shadow;
        self
    }

    pub fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = Some(tooltip.into());
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

/// The button floating over a list scrolled away from what is playing, taking it back there.
#[derive(IntoElement)]
pub struct FollowButton {
    id: SharedString,
    click: Option<OnClick>,
}

impl FollowButton {
    pub fn new(id: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            click: None,
        }
    }

    pub fn on_click(
        mut self,
        click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.click = Some(Rc::new(click));
        self
    }
}

impl RenderOnce for FollowButton {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let mut button = IconButton::new(self.id, icons::REFRESH)
            .size(style::chrome().round_button)
            .glyph(px(20.))
            .tone(IconTone::Scrim)
            .shadow(true)
            .stop_propagation(true)
            .tooltip(crate::format::t("jump_to_current_song"));
        button.click = self.click;
        div().absolute().bottom_4().right_4().child(button)
    }
}

impl RenderOnce for IconButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self.id;
        let (hovered, pointer) = motion::hovered(motion::channel(&id, HOVER), window, cx);
        let hot = hovered && !self.disabled;
        let (fill, ink, ring) = self.tone.colours(hot, self.lit, self.accent, cx);
        let fill = motion::color(motion::channel(&id, WASH), fill, window, cx);
        let ink = motion::color(motion::channel(&id, INK), ink, window, cx);
        let edge = self.ring.or(ring).map(|(rest, lifted)| {
            let alpha = if hot { lifted } else { rest };
            motion::color(
                motion::channel(&id, EDGE),
                style::white(alpha, cx),
                window,
                cx,
            )
        });
        let glyph = self.glyph.unwrap_or(self.size * 0.36);
        let arriving = match self.swap {
            Some((_, shown)) => ink.opacity(ink.a * shown),
            None => ink,
        };
        // A host wraps the button in a div that shrinks to it, so the button carries the frame's size itself.
        let inner = self.size - if edge.is_some() { RING * 2. } else { px(0.) };
        let stop = self.stop;
        let (width, corner) = match self.shape {
            Some(shape) => (self.size * shape.width, self.size * shape.corner),
            None => (self.size, self.size / 2.),
        };
        let inner_width = inner + (width - self.size);
        let button = Button::new(SharedString::from(format!("{id}-button")))
            .custom(style::bare_button(arriving, cx))
            .icon(icons::icon(self.icon))
            .with_size(glyph / 0.75)
            .size(inner)
            .w(inner_width)
            .rounded(corner)
            .disabled(self.disabled)
            .loading(self.loading)
            .when_some(self.tooltip, |button, tip| button.tooltip(tip))
            .when_some(self.click, |button, click| {
                button.on_click(move |event, window, cx| {
                    if stop {
                        cx.stop_propagation();
                    }
                    click(event, window, cx);
                })
            });
        let button = focus::quiet(button, cx);
        let control = match self.host {
            Some(host) => host(button),
            None => button.into_any_element(),
        };
        hover::framed(&id, pointer, control)
            .flex_shrink_0()
            .h(self.size)
            .w(width)
            .rounded(corner)
            .bg(fill)
            .when_some(edge, |frame, edge| frame.border_1().border_color(edge))
            .when(self.disabled, |frame| frame.opacity(DISABLED))
            .when(self.shadow, |frame| frame.shadow_md())
            .when_some(self.swap, |frame, (from, shown)| {
                frame.relative().child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            icons::icon(from)
                                .size(glyph)
                                .text_color(ink.opacity(ink.a * (1. - shown))),
                        ),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::component::Root;
    use gpui_kit::component::menu::DropdownMenu as _;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, Context, Render, TestAppContext, point, size};

    struct Fixture {
        tone: IconTone,
        hosted: bool,
    }

    impl Render for Fixture {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let button = IconButton::new("dots", icons::MORE)
                .size(px(40.))
                .tone(self.tone);
            let button = match self.hosted {
                true => {
                    button.host(|button| button.dropdown_menu(|menu, _, _| menu).into_any_element())
                }
                false => button,
            };
            div().size(px(200.)).child(button)
        }
    }

    #[gpui_kit::test]
    fn a_hosted_button_fills_its_circle_like_a_bare_one(cx: &mut TestAppContext) {
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
        for (tone, inset) in [(IconTone::Quiet, 0.), (IconTone::Outline, 1.)] {
            for hosted in [false, true] {
                let view = cx.add_window(move |window, cx| {
                    let fixture = cx.new(|_| Fixture { tone, hosted });
                    Root::new(fixture, window, cx)
                });
                cx.update_window(view.into(), |_, window, cx| {
                    window.render_frame(cx);
                    let bounds = window.find("dots-button").bounds();
                    assert_eq!(
                        bounds.origin,
                        point(px(inset), px(inset)),
                        "{tone:?} {hosted}"
                    );
                    let side = px(40. - inset * 2.);
                    assert_eq!(bounds.size, size(side, side), "{tone:?} {hosted}");
                })
                .unwrap();
            }
        }
    }
}
