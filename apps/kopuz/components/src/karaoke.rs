//! Clip the sung layer against shaped text, preserving wrapping and syllable timing.

use std::ops::Range;

use gpui_kit::{
    App, Bounds, ContentMask, Element, ElementId, GlobalElementId, HighlightStyle, Hsla,
    InspectorElementId, IntoElement, LayoutId, Pixels, SharedString, StyledText, TextAlign,
    TextRun, Window, WrappedLine, point, px, size,
};

// The soft band either side of the wipe's edge, as a share of the syllable: the Dioxus gradient's 8% of 220%.
const BAND: f32 = 0.088;
// GPUI paints with no per-layer opacity, so the band is stepped through this many translucent slices.
const FEATHER_STEPS: usize = 5;
// `0 0 (4 + glow * 6)px rgba(255,255,255,glow * 0.3)`, stood in for by copies of the glyphs set around them.
const GLOW_ALPHA: f32 = 0.3;
const GLOW_RADIUS: (f32, f32) = (1.5, 2.5);
const GLOW_OFFSETS: [(f32, f32); 8] = [
    (1., 0.),
    (-1., 0.),
    (0., 1.),
    (0., -1.),
    (0.7, 0.7),
    (-0.7, 0.7),
    (0.7, -0.7),
    (-0.7, -0.7),
];

/// One sung word or syllable: its byte range of the line, how far the voice is through it, and how brightly it glows.
#[derive(Debug, Clone, PartialEq)]
pub struct Syllable {
    pub range: Range<usize>,
    pub fill: f32,
    pub glow: f32,
}

/// One visual row of the wrapped text: its byte range, shaped line, and where it is drawn.
struct Row {
    range: Range<usize>,
    shaped: std::sync::Arc<gpui_kit::LineLayout>,
    left: Pixels,
    x: Pixels,
    y: Pixels,
    offset: usize,
}

pub struct KaraokeText {
    text: StyledText,
    source: SharedString,
    color: Hsla,
    bright: Vec<WrappedLine>,
    run: Option<(Pixels, Option<Pixels>, TextRun)>,
    syllables: Vec<Syllable>,
}

impl KaraokeText {
    pub fn new(source: SharedString, color: Hsla, dim: Hsla, syllables: Vec<Syllable>) -> Self {
        let text = StyledText::new(source.clone()).with_highlights([(
            0..source.len(),
            HighlightStyle {
                color: Some(dim),
                ..Default::default()
            },
        )]);
        Self {
            text,
            source,
            color,
            bright: Vec::new(),
            run: None,
            syllables,
        }
    }

    fn rows(&self, align: TextAlign) -> (Vec<Row>, Pixels) {
        let layout = self.text.layout();
        let bounds = layout.bounds();
        let height = layout.line_height();
        let mut rows = Vec::new();
        let mut offset = 0;
        let mut y = bounds.origin.y;
        for line in layout.line_layouts() {
            let shaped = &line.unwrapped_layout;
            let ends = line
                .wrap_boundaries
                .iter()
                .map(|boundary| shaped.runs[boundary.run_ix].glyphs[boundary.glyph_ix].index)
                .chain([line.len()]);
            let mut start = 0;
            for end in ends {
                let left = shaped.x_for_index(start);
                let width = shaped.x_for_index(end) - left;
                let inset = match align {
                    TextAlign::Center => (bounds.size.width - width) / 2.,
                    TextAlign::Right => bounds.size.width - width,
                    _ => px(0.),
                };
                rows.push(Row {
                    range: offset + start..offset + end,
                    shaped: shaped.clone(),
                    left,
                    x: bounds.origin.x + inset,
                    y,
                    offset,
                });
                start = end;
                y += height;
            }
            offset += line.len() + 1;
        }
        (rows, height)
    }

    /// Where a syllable's glyphs sit, one box per row it spans.
    fn segments(range: &Range<usize>, rows: &[Row], height: Pixels) -> Vec<Bounds<Pixels>> {
        let mut segments = Vec::new();
        for row in rows {
            let start = range.start.max(row.range.start);
            let end = range.end.min(row.range.end);
            if start >= end {
                continue;
            }
            let from = row.shaped.x_for_index(start - row.offset);
            let to = row.shaped.x_for_index(end - row.offset);
            if to > from {
                segments.push(Bounds::new(
                    point(row.x + from - row.left, row.y),
                    size(to - from, height),
                ));
            }
        }
        segments
    }

    /// The lit boxes and the alpha each is painted at: solid up to the wipe's edge, then a stepped feather.
    fn masks(&self, rows: &[Row], height: Pixels) -> Vec<(Bounds<Pixels>, f32)> {
        let mut masks = Vec::new();
        for syllable in &self.syllables {
            if syllable.fill <= 0. {
                continue;
            }
            let segments = Self::segments(&syllable.range, rows, height);
            for (from, to, alpha) in feather(syllable.fill) {
                masks.extend(
                    span_segments(&segments, from, to)
                        .into_iter()
                        .map(|mask| (mask, alpha)),
                );
            }
        }
        masks
    }

    fn shape(&self, color: Hsla, window: &mut Window) -> Vec<WrappedLine> {
        let Some((font_size, wrap_width, run)) = self.run.clone() else {
            return Vec::new();
        };
        let run = TextRun { color, ..run };
        window
            .text_system()
            .shape_text(self.source.clone(), font_size, &[run], wrap_width, None)
            .map(|lines| lines.into_vec())
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "could not shape karaoke text");
                Vec::new()
            })
    }
}

/// The share of a syllable lit by `fill`, as (from, to, alpha) spans along it.
fn feather(fill: f32) -> Vec<(f32, f32, f32)> {
    let edge = -BAND + (1. + 2. * BAND) * fill.clamp(0., 1.);
    let mut spans = vec![(0., edge - BAND, 1.)];
    let step = 2. * BAND / FEATHER_STEPS as f32;
    for index in 0..FEATHER_STEPS {
        let from = edge - BAND + step * index as f32;
        let alpha = 1. - (index as f32 + 0.5) / FEATHER_STEPS as f32;
        spans.push((from, from + step, alpha));
    }
    spans
        .into_iter()
        .map(|(from, to, alpha)| (from.clamp(0., 1.), to.clamp(0., 1.), alpha))
        .filter(|(from, to, _)| to > from)
        .collect()
}

/// The part of the row boxes lying between `from` and `to`, as shares of their combined width.
fn span_segments(segments: &[Bounds<Pixels>], from: f32, to: f32) -> Vec<Bounds<Pixels>> {
    let total = segments
        .iter()
        .map(|segment| segment.size.width)
        .sum::<Pixels>();
    let (from, to) = (total * from, total * to);
    let mut walked = px(0.);
    segments
        .iter()
        .filter_map(|segment| {
            let start = walked;
            walked += segment.size.width;
            let left = from.max(start);
            let right = to.min(walked);
            (right > left).then(|| {
                Bounds::new(
                    point(segment.origin.x + left - start, segment.origin.y),
                    size(right - left, segment.size.height),
                )
            })
        })
        .collect()
}

#[cfg(test)]
fn filled_segments(segments: &[Bounds<Pixels>], fill: f32) -> Vec<Bounds<Pixels>> {
    span_segments(segments, 0., fill.clamp(0., 1.))
}

impl IntoElement for KaraokeText {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for KaraokeText {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        self.text.request_layout(id, inspector, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.text.prepaint(id, inspector, bounds, state, window, cx);
        let layouts = self.text.layout().line_layouts();
        let Some(first) = layouts.first() else { return };
        let mut style = window.text_style();
        style.color = self.color;
        let run = style.to_run(self.source.len());
        self.run = Some((first.font_size(), first.wrap_width, run));
        self.bright = self.shape(self.color, window);
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut (),
        prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.text
            .paint(id, inspector, bounds, state, prepaint, window, cx);
        let layout = self.text.layout();
        let text_bounds = layout.bounds();
        let line_height = layout.line_height();
        let align = window.text_style().text_align;
        let (rows, height) = self.rows(align);
        let paint = |lines: &[WrappedLine],
                     nudge: gpui_kit::Point<Pixels>,
                     mask,
                     window: &mut Window,
                     cx: &mut App| {
            window.with_content_mask(Some(ContentMask { bounds: mask }), |window| {
                let mut origin = text_bounds.origin + nudge;
                for line in lines {
                    if let Err(error) =
                        line.paint(origin, line_height, align, Some(text_bounds), window, cx)
                    {
                        tracing::warn!(%error, "could not paint karaoke text");
                    }
                    origin.y += line.size(line_height).height;
                }
            });
        };

        // The glow sits under the glyphs, so it is laid down before the fill.
        for syllable in self.syllables.iter().filter(|syllable| syllable.glow > 0.) {
            let glow = syllable.glow.min(1.);
            let radius = px(GLOW_RADIUS.0 + GLOW_RADIUS.1 * glow);
            let ink = self.color.alpha(self.color.a * GLOW_ALPHA * glow / 3.);
            let halo = self.shape(ink, window);
            for segment in Self::segments(&syllable.range, &rows, height) {
                let mask = Bounds::new(
                    segment.origin - point(radius * 2., radius * 2.),
                    segment.size + size(radius * 4., radius * 4.),
                );
                for (dx, dy) in GLOW_OFFSETS {
                    paint(&halo, point(radius * dx, radius * dy), mask, window, cx);
                }
            }
        }

        let masks = self.masks(&rows, height);
        // The solid fill uses the layer shaped in prepaint; each feather step gets one of its own.
        let mut layers: Vec<(f32, Vec<WrappedLine>)> = Vec::new();
        for (mask, alpha) in masks {
            if alpha >= 1. {
                paint(&self.bright, point(px(0.), px(0.)), mask, window, cx);
                continue;
            }
            let index = match layers.iter().position(|(held, _)| *held == alpha) {
                Some(index) => index,
                None => {
                    let lines = self.shape(self.color.alpha(self.color.a * alpha), window);
                    layers.push((alpha, lines));
                    layers.len() - 1
                }
            };
            paint(&layers[index].1, point(px(0.), px(0.)), mask, window, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wrapped_word_fills_across_rows_without_lighting_the_next_row_early() {
        let rows = [
            Bounds::new(point(px(10.), px(20.)), size(px(60.), px(24.))),
            Bounds::new(point(px(5.), px(44.)), size(px(40.), px(24.))),
        ];
        let halfway = filled_segments(&rows, 0.5);
        assert_eq!(halfway.len(), 1);
        assert_eq!(halfway[0].size.width, px(50.));
        let later = filled_segments(&rows, 0.8);
        assert_eq!(later[0], rows[0]);
        assert_eq!(later[1].origin, rows[1].origin);
        assert_eq!(later[1].size.width, px(20.));
        assert!(filled_segments(&rows, 0.).is_empty());
        assert_eq!(filled_segments(&rows, 1.), rows);
    }

    #[test]
    fn the_wipe_feathers_its_edge_and_lands_whole() {
        assert!(feather(0.).is_empty());
        let halfway = feather(0.5);
        assert_eq!(halfway[0].2, 1.);
        assert!(halfway.windows(2).all(|pair| pair[1].2 < pair[0].2));
        assert_eq!(feather(1.)[0], (0., 1., 1.));
    }

    struct Fixture {
        text: gpui_kit::SharedString,
        fill: f32,
        layout: std::rc::Rc<std::cell::RefCell<Option<gpui_kit::TextLayout>>>,
    }

    impl gpui_kit::Render for Fixture {
        fn render(&mut self, _: &mut Window, _: &mut gpui_kit::Context<Self>) -> impl IntoElement {
            use gpui_kit::{ParentElement as _, Styled as _};
            let karaoke = KaraokeText::new(
                self.text.clone(),
                gpui_kit::white(),
                gpui_kit::black(),
                vec![Syllable {
                    range: 0..self.text.len(),
                    fill: self.fill,
                    glow: self.fill,
                }],
            );
            *self.layout.borrow_mut() = Some(karaoke.text.layout().clone());
            gpui_kit::div()
                .w(px(160.))
                .text_size(px(24.))
                .text_center()
                .child(karaoke)
        }
    }

    #[gpui_kit::test]
    fn native_text_keeps_identical_wrapping_as_words_fill(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        let layout = std::rc::Rc::new(std::cell::RefCell::new(None));
        let view = cx.add_window({
            let layout = layout.clone();
            move |_, _| Fixture {
                text: "深海 52Hz — words across several wrapped lines".into(),
                fill: 0.,
                layout,
            }
        });
        let before = cx
            .update_window(view.into(), |_, window, cx| {
                window.draw(cx).clear(cx);
                let layout = layout.borrow();
                let layout = layout.as_ref().unwrap();
                (layout.bounds(), layout.wrapped_text())
            })
            .unwrap();
        cx.update_window(view.into(), |root, window, cx| {
            root.downcast::<Fixture>().unwrap().update(cx, |view, cx| {
                view.fill = 0.5;
                cx.notify();
            });
            window.draw(cx).clear(cx);
        })
        .unwrap();
        let layout = layout.borrow();
        let layout = layout.as_ref().unwrap();
        assert_eq!(layout.bounds(), before.0);
        assert_eq!(layout.wrapped_text(), before.1);
        assert!(layout.bounds().size.height > layout.line_height());
    }
}
