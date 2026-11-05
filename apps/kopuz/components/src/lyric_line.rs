//! One line of synced lyrics: its colouring, its karaoke fill or interlude bar, and the seek its click asks for.

use std::rc::Rc;

use gpui_kit::base::StyledExt as _;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, Pixels, RenderOnce,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px, relative, rems,
};

use crate::{KaraokeText, Syllable, icons, motion};

// A background line is quieter than a main one even once it is lit.
const BACKGROUND_ALPHA: f32 = 0.7;
const INTERLUDE_BAR: gpui_kit::Pixels = px(72.);
// `text-white/40` unsung, `text-white/60` under the pointer: a line the voice has left.
const DIM: f32 = 0.4;
const DIM_HOVER: f32 = 0.6;
// What a lit line's words wear before the voice reaches them: the Dioxus wipe's unsung 45%.
const UNSUNG: f32 = 0.45;
// A background line is quieter still: `text-white/25`, lit at `text-white/70`.
const BACKGROUND_DIM: f32 = 0.25;
// `min(100%, 20rem)`, and `min(90%, 18rem)` once a duet pushes the lines to the edges.
const LINE_WIDTH: f32 = 20.;
const TURN_WIDTH: f32 = 18.;
// `leading-snug` on the small print of a background line.
const SNUG: f32 = 1.375;
// GPUI filters no text, so a blur is sampled: the words again around one ring, each copy faint.
const BLUR_TAPS: usize = 6;
// Weights of the ink kept at the centre and given to each tap; together under one, so a blurred line reads lighter.
const BLUR_CENTRE: f32 = 0.35;
const BLUR_TAP: f32 = 0.05;
// Wider than this the copies part into ghost lines instead of a blur.
const BLUR_RING_MAX: Pixels = px(8.);
/// How much of a sharp line's ink a blurred one shows, whatever its radius.
pub const BLURRED_INK: f32 = BLUR_CENTRE + BLUR_TAPS as f32 * BLUR_TAP;

type Seek = Rc<dyn Fn(u64, &mut Window, &mut App)>;

/// What a line shows besides its words.
enum Body {
    Words,
    /// Per-syllable fills and glows over the line's text.
    Karaoke(Vec<Syllable>),
    /// An instrumental stretch: a bar that fills over it, 0..1.
    Interlude(f32),
}

/// A line of lyrics, drawn the same whether it is dim, lit, sung syllable by syllable or an interlude.
#[derive(IntoElement)]
pub struct LyricLine {
    index: usize,
    start_ms: u64,
    text: SharedString,
    body: Body,
    lit: bool,
    background: bool,
    opposite: bool,
    paired: bool,
    opacity: f32,
    blur: Pixels,
    ink: Hsla,
    seek: Option<Seek>,
}

impl LyricLine {
    /// A line at `index` of the list, which starts `start_ms` into the track.
    pub fn new(index: usize, start_ms: u64, text: impl Into<SharedString>) -> Self {
        Self {
            index,
            start_ms,
            text: text.into(),
            body: Body::Words,
            lit: false,
            background: false,
            opposite: false,
            paired: false,
            opacity: 1.,
            blur: px(0.),
            ink: Hsla::default(),
            seek: None,
        }
    }

    /// The colour the voice leaves behind; the dim and the lit states are derived from it.
    pub fn ink(mut self, ink: Hsla) -> Self {
        self.ink = ink;
        self
    }

    pub fn lit(mut self, lit: bool) -> Self {
        self.lit = lit;
        self
    }

    /// A backing vocal: smaller and quieter than the line it sits under.
    pub fn background(mut self, background: bool) -> Self {
        self.background = background;
        self
    }

    /// The other singer's turn, set to the right.
    pub fn opposite(mut self, opposite: bool) -> Self {
        self.opposite = opposite;
        self
    }

    /// Some line of the song is the other singer's, so this one sets to the left rather than centred.
    pub fn paired(mut self, paired: bool) -> Self {
        self.paired = paired;
        self
    }

    /// How far the line has faded from the one being sung.
    pub fn opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity;
        self
    }

    /// How far out of focus the line has receded from the one being sung; a lit line stays sharp.
    pub fn blur(mut self, radius: Pixels) -> Self {
        self.blur = radius;
        self
    }

    /// Light the line's syllables by the given fills; only a lit line is drawn with them.
    pub fn karaoke(mut self, fills: Vec<Syllable>) -> Self {
        self.body = Body::Karaoke(fills);
        self
    }

    /// An instrumental stretch with `progress` of it played.
    pub fn interlude(mut self, progress: f32) -> Self {
        self.body = Body::Interlude(progress);
        self
    }

    /// What a click does: the line's start, in milliseconds, for the caller to seek to.
    pub fn on_seek(mut self, seek: impl Fn(u64, &mut Window, &mut App) + 'static) -> Self {
        self.seek = Some(Rc::new(seek));
        self
    }
}

impl RenderOnce for LyricLine {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let (lit, background, opposite, paired) =
            (self.lit, self.background, self.opposite, self.paired);
        let sung = self.ink;
        let tint = if background { BACKGROUND_ALPHA } else { 1.0 };
        let base = if lit {
            sung.alpha(tint)
        } else if background {
            sung.alpha(BACKGROUND_DIM)
        } else {
            sung.alpha(DIM)
        };
        let turned = opposite || paired;
        let (start_ms, seek) = (self.start_ms, self.seek);
        let hover = sung.alpha(DIM_HOVER * tint);
        let blur = Some(self.blur).filter(|radius| !lit && *radius > px(0.));
        let group = SharedString::from(format!("lyric-{}", self.index));

        let body = div()
            .id(("lyric", self.index))
            .w(if turned { relative(0.9) } else { relative(1.) })
            .max_w(rems(if turned { TURN_WIDTH } else { LINE_WIDTH }))
            .py_0p5()
            .opacity(self.opacity)
            .group(group.clone())
            .text_color(base)
            .when(background, |row| {
                row.pl_4()
                    .text_sm()
                    .font_medium()
                    .line_height(relative(SNUG))
            })
            .when(!background, |row| row.font_semibold())
            // The `scale(1.12)` the lit Dioxus line grows by is a transform GPUI has not; a size step stands in.
            .when(!background && lit, |row| row.text_xl())
            .when(!background && !lit, |row| row.text_lg())
            .when(opposite, |row| row.text_right().italic().ml_auto())
            .when(!opposite && paired, |row| row.text_left())
            .when(!opposite && !paired, |row| row.text_center().mx_auto())
            .when(!lit, |row| row.hover(move |row| row.text_color(hover)))
            .on_click(move |_, window, cx| {
                if let Some(seek) = &seek {
                    seek(start_ms, window, cx);
                }
            });

        match self.body {
            Body::Interlude(progress) => body
                .flex()
                .py_2()
                .items_center()
                .gap_2()
                .when(!paired, |row| row.justify_center())
                .child(icons::icon(icons::MUSIC).size(px(16.)))
                .child(
                    div()
                        .w(INTERLUDE_BAR)
                        .h(px(3.))
                        .rounded_full()
                        .bg(cx.theme().muted)
                        .child(
                            div()
                                .h(px(3.))
                                .w(INTERLUDE_BAR * progress.clamp(0., 1.))
                                .rounded_full()
                                .bg(sung.alpha(tint)),
                        ),
                )
                .into_any_element(),
            Body::Karaoke(fills) if lit => body
                .child(KaraokeText::new(
                    self.text,
                    sung.alpha(tint),
                    sung.alpha(UNSUNG * tint),
                    fills,
                ))
                .into_any_element(),
            Body::Karaoke(_) | Body::Words => match blur {
                Some(radius) => body
                    .child(blurred(self.text, base, hover, radius, group))
                    .into_any_element(),
                None => body.child(self.text).into_any_element(),
            },
        }
    }
}

/// The words drawn out of focus by `radius`, coming sharp again while the pointer is on the line.
fn blurred(
    text: SharedString,
    ink: Hsla,
    hover: Hsla,
    radius: Pixels,
    group: SharedString,
) -> impl IntoElement {
    let ring = ink.alpha(ink.a * BLUR_TAP);
    let radius = radius.min(BLUR_RING_MAX);
    let copies = (0..BLUR_TAPS).map(move |tap| {
        let angle = std::f32::consts::TAU * tap as f32 / BLUR_TAPS as f32;
        (radius * angle.cos(), radius * angle.sin())
    });
    div()
        .relative()
        .w_full()
        .child(
            div()
                .text_color(ink.alpha(ink.a * BLUR_CENTRE))
                .group_hover(group.clone(), move |style| style.text_color(hover))
                .child(text.clone()),
        )
        .children(copies.map(move |(x, y)| {
            div()
                .absolute()
                .top(y)
                .left(x)
                .w_full()
                .text_color(ring)
                .group_hover(group.clone(), |style| style.opacity(0.))
                .child(text.clone())
        }))
}

/// The text colour a lyric wears when the voice is on it: the sheet's accent, held legible.
pub fn sung_ink(window: &mut Window, cx: &mut App) -> Hsla {
    let base = cx.theme().foreground;
    let background = cx.theme().background;
    theme::readable_on(
        motion::accent(theme::Surface::Sheet, base, window, cx),
        background,
    )
}
