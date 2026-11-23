//! One line of synced lyrics: its colouring, its karaoke fill or interlude bar, and the seek its click asks for.

use std::rc::Rc;
use std::time::Duration;

use gpui_kit::base::StyledExt as _;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Animation, AnimationExt as _, App, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, RenderOnce, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, div, ease_out_quint, px, relative, rems,
};

use crate::{KaraokeText, Syllable, icons, motion};

// A background line is quieter than a main one even once it is lit.
const BACKGROUND_ALPHA: f32 = 0.7;
const INTERLUDE_BAR: gpui_kit::Pixels = px(72.);
// A line the voice has left, and under the pointer; above the Dioxus `text-white/40` so it reads on light themes too.
const DIM: f32 = 0.6;
const DIM_HOVER: f32 = 0.85;
// What a lit line's words wear before the voice reaches them: the Dioxus wipe's unsung 45%.
const UNSUNG: f32 = 0.45;
// A background line is quieter still.
const BACKGROUND_DIM: f32 = 0.45;
// However the palette sits, a resting line keeps WCAG's large-text ratio and a backing vocal a little under it.
const DIM_RATIO: f32 = theme::AA_LARGE;
const BACKGROUND_RATIO: f32 = 2.4;
// The Dioxus lit line's `scale(1.12)`, and `scale(1.06)` once a duet pushes the lines to the edges.
const LIT_SCALE: f32 = 1.12;
const TURN_LIT_SCALE: f32 = 1.06;
// `leading-snug` on the small print of a background line.
const SNUG: f32 = 1.375;
// A romanized reading sits under its line, quieter than the words it reads.
const READING_ALPHA: f32 = 0.7;
// The opacity a line becomes lit from, the Dioxus `fadeLineIn`.
const LIGHT_FROM: f32 = 0.68;
const LIGHT_UP: Duration = Duration::from_millis(260);
// GPUI filters no text, so a blur is sampled: the words again around two rings, each copy faint.
const BLUR_TAPS: usize = 6;
// Weights of the ink kept at the centre and given to each tap; near one, so a blurred line stays as legible as a CSS blur.
const BLUR_CENTRE: f32 = 0.42;
const BLUR_INNER_TAP: f32 = 0.05;
const BLUR_TAP: f32 = 0.035;
// Wider than this the copies part into ghost lines instead of a blur.
const BLUR_RING_MAX: Pixels = px(16.);
/// How much of a sharp line's ink a blurred one shows, whatever its radius.
pub const BLURRED_INK: f32 = BLUR_CENTRE + BLUR_TAPS as f32 * (BLUR_INNER_TAP + BLUR_TAP);

/// Type and widths, in rems, for the docked panel and for the sheet's larger stage.
struct Measures {
    text: f32,
    background: f32,
    reading: f32,
    width: f32,
    turn_width: f32,
    note: Pixels,
}

// `text-lg` lines in `min(100%, 20rem)`, backing vocals `text-sm`, readings `text-sm`.
const PANEL: Measures = Measures {
    text: 1.125,
    background: 0.875,
    reading: 0.875,
    width: 20.,
    turn_width: 18.,
    note: px(20.),
};
// `text-2xl` lines in `min(100%, 38rem)`, backing vocals `text-xl`, readings `text-base`.
const SHEET: Measures = Measures {
    text: 1.5,
    background: 1.25,
    reading: 1.,
    width: 38.,
    turn_width: 34.,
    note: px(28.),
};

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
    reading: Option<SharedString>,
    body: Body,
    lit: bool,
    background: bool,
    opposite: bool,
    paired: bool,
    large: bool,
    opacity: f32,
    blur: Pixels,
    ink: Hsla,
    scale: f32,
    seek: Option<Seek>,
}

impl LyricLine {
    /// A line at `index` of the list, which starts `start_ms` into the track.
    pub fn new(index: usize, start_ms: u64, text: impl Into<SharedString>) -> Self {
        Self {
            index,
            start_ms,
            text: text.into(),
            reading: None,
            body: Body::Words,
            lit: false,
            background: false,
            opposite: false,
            paired: false,
            large: false,
            opacity: 1.,
            blur: px(0.),
            ink: Hsla::default(),
            scale: 1.,
            seek: None,
        }
    }

    /// The reader's lyrics size, as a factor of the layout's own type.
    pub fn scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }

    /// The colour of the line being sung; the others take the theme's text colour.
    pub fn ink(mut self, ink: Hsla) -> Self {
        self.ink = ink;
        self
    }

    /// A Latin reading of the words, drawn smaller and dimmer under them.
    pub fn reading(mut self, reading: Option<SharedString>) -> Self {
        self.reading = reading;
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

    /// Set at the sheet's larger size rather than the docked panel's.
    pub fn large(mut self, large: bool) -> Self {
        self.large = large;
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
        let large = self.large;
        let measures = if large { &SHEET } else { &PANEL };
        let sung = self.ink;
        // The accent can sit close to the backdrop (gruvbox's does), so the lines it is not on wear the text colour.
        let rest = cx.theme().foreground;
        let backdrop = cx.theme().background;
        let tint = if background { BACKGROUND_ALPHA } else { 1.0 };
        let base = if lit {
            sung.alpha(tint)
        } else if background {
            resting_ink(rest, backdrop, BACKGROUND_DIM, BACKGROUND_RATIO)
        } else {
            resting_ink(rest, backdrop, DIM, DIM_RATIO)
        };
        let turned = opposite || paired;
        let (start_ms, seek) = (self.start_ms, self.seek);
        let hover = resting_ink(rest, backdrop, DIM_HOVER * tint, DIM_RATIO);
        let blur = Some(self.blur).filter(|radius| !lit && *radius > px(0.));
        let group = SharedString::from(format!("lyric-{}", self.index));
        let scale = self.scale;
        let size = scale
            * if background {
                measures.background
            } else if lit && turned {
                measures.text * TURN_LIT_SCALE
            } else if lit {
                measures.text * LIT_SCALE
            } else {
                measures.text
            };

        let body = div()
            .id(("lyric", self.index))
            .w(if turned { relative(0.9) } else { relative(1.) })
            .max_w(rems(if turned {
                measures.turn_width
            } else {
                measures.width
            }))
            .py_0p5()
            .opacity(self.opacity)
            .cursor_pointer()
            .group(group.clone())
            .text_color(base)
            .text_size(rems(size))
            .when(background, |row| {
                row.font_medium().line_height(relative(SNUG))
            })
            .when(!background, |row| row.font_semibold())
            .when(!background && opposite, |row| row.italic())
            .when(opposite, |row| row.text_right().ml_auto())
            .when(!opposite && paired, |row| row.text_left())
            .when(!opposite && !paired, |row| row.text_center().mx_auto())
            // A backing vocal steps in from the edge its singer's line sits on; centred, it needs no step.
            .when(background && opposite, |row| {
                if large { row.pr_6() } else { row.pr_4() }
            })
            .when(background && paired && !opposite, |row| {
                if large { row.pl_6() } else { row.pl_4() }
            })
            .when(!lit, |row| row.hover(move |row| row.text_color(hover)))
            .on_click(move |_, window, cx| {
                if let Some(seek) = &seek {
                    seek(start_ms, window, cx);
                }
            });

        let reading = self.reading.map(|reading| {
            let ink = base.alpha(base.a * READING_ALPHA);
            let line = div()
                .text_size(rems(measures.reading * scale))
                .font_medium()
                .not_italic()
                .line_height(relative(SNUG))
                .text_color(ink);
            match blur {
                Some(radius) => line
                    .child(blurred(reading, ink, hover, radius, group.clone()))
                    .into_any_element(),
                None => line.child(reading).into_any_element(),
            }
        });

        let row = match self.body {
            Body::Interlude(progress) => body
                .flex()
                .py_2()
                .items_center()
                .gap_2()
                .when(!paired, |row| row.justify_center())
                .child(icons::icon(icons::MUSIC).size(measures.note * scale))
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
                ),
            Body::Karaoke(fills) if lit => body
                .child(KaraokeText::new(
                    self.text,
                    sung.alpha(tint),
                    sung.alpha(UNSUNG * tint),
                    fills,
                ))
                .children(reading),
            Body::Karaoke(_) | Body::Words => match blur {
                Some(radius) => body
                    .child(blurred(self.text, base, hover, radius, group))
                    .children(reading),
                None => body.child(self.text).children(reading),
            },
        };
        match motion::span(LIGHT_UP, cx).filter(|_| lit) {
            Some(duration) => row
                .with_animation(
                    ("lyric-lit", self.index),
                    Animation::new(duration).with_easing(ease_out_quint()),
                    |row, delta| row.opacity(LIGHT_FROM + (1. - LIGHT_FROM) * delta),
                )
                .into_any_element(),
            None => row.into_any_element(),
        }
    }
}

/// `alpha` of the text colour laid over the backdrop, pushed until it holds `ratio` against it.
fn resting_ink(text: Hsla, backdrop: Hsla, alpha: f32, ratio: f32) -> Hsla {
    let backdrop = backdrop.alpha(1.);
    theme::readable_at(backdrop.blend(text.alpha(text.a * alpha)), backdrop, ratio)
}

/// The words drawn out of focus by `radius`, coming sharp again while the pointer is on the line.
fn blurred(
    text: SharedString,
    ink: Hsla,
    hover: Hsla,
    radius: Pixels,
    group: SharedString,
) -> impl IntoElement {
    let radius = radius.min(BLUR_RING_MAX);
    let rings = [
        (radius / 2., ink.alpha(ink.a * BLUR_INNER_TAP), 0.5),
        (radius, ink.alpha(ink.a * BLUR_TAP), 0.),
    ];
    let copies = rings.into_iter().flat_map(move |(reach, ring, turn)| {
        (0..BLUR_TAPS).map(move |tap| {
            let angle = std::f32::consts::TAU * (tap as f32 + turn) / BLUR_TAPS as f32;
            (reach * angle.cos(), reach * angle.sin(), ring)
        })
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
        .children(copies.map(move |(x, y, ring)| {
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::hsla;

    #[test]
    fn a_resting_line_holds_the_large_text_ratio_on_light_and_dark_backdrops() {
        let themes = [
            (hsla(0., 0., 0.95, 1.), hsla(0., 0., 0.07, 1.)),
            (hsla(0., 0., 0.1, 1.), hsla(0., 0., 0.98, 1.)),
            (hsla(0.1, 0.3, 0.85, 1.), hsla(0.1, 0.2, 0.2, 1.)),
        ];
        for (text, backdrop) in themes {
            let ink = resting_ink(text, backdrop, DIM, DIM_RATIO);
            assert!(theme::contrast(ink, backdrop) >= DIM_RATIO - 0.01);
            let backing = resting_ink(text, backdrop, BACKGROUND_DIM, BACKGROUND_RATIO);
            assert!(theme::contrast(backing, backdrop) >= BACKGROUND_RATIO - 0.01);
        }
    }
}
