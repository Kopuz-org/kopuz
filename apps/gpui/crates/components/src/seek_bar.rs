//! The seek bar with its pointer preview, and the clocks that hold still either side of it.

use std::sync::{Arc, OnceLock};

use api::BufferedRange;
use gpui_kit::base::slider::SliderState;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Entity, FontFeatures, Hsla, InteractiveElement as _, IntoElement, MouseMoveEvent,
    ParentElement as _, Pixels, Rems, RenderOnce, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, div, px, rems,
};

use crate::format::duration;
use crate::thin_slider::ThinSlider;
use crate::{motion, style};

const PREVIEW: u64 = 1;
const REMAINING: u64 = 2;

// The paint, and the hit area around it: a 3px target is not a target.
const BAR: Pixels = px(3.);
const HIT: Pixels = px(18.);
const KNOB: Pixels = px(12.);
// The floating time that follows the pointer, and how far above the bar it sits.
const PREVIEW_LIFT: Pixels = px(26.);
const PREVIEW_WIDTH: Pixels = px(56.);

// Two ranges this close read as one bar, so the seek track does not look shredded.
const MERGE_GAP_PERCENT: f32 = 0.75;
// A range that all but touches the playhead is pulled onto it, so no sliver of track shows through.
const PLAYHEAD_SNAP_PERCENT: f32 = 5.0;

const HOUR_MS: u64 = 3_600_000;
// `4:33` and `1:04:33` at the sizes the clocks are drawn.
const SHORT: f32 = 2.5;
const LONG: f32 = 4.;

/// Buffered byte ranges as `(left, right)` percentages, merged and snapped the way the bar draws them.
pub fn visual_ranges(ranges: &[BufferedRange], played_percent: f32) -> Vec<(f32, f32)> {
    let mut visual: Vec<(f32, f32)> = Vec::with_capacity(ranges.len());
    for range in ranges {
        let Some(total) = range.total.filter(|total| *total > 0) else {
            continue;
        };
        let start = (range.start as f32 / total as f32 * 100.0).clamp(0.0, 100.0);
        let end = (range.end as f32 / total as f32 * 100.0).clamp(start, 100.0);
        match visual.last_mut() {
            Some(previous) if start - previous.1 <= MERGE_GAP_PERCENT => {
                previous.1 = previous.1.max(end)
            }
            _ => visual.push((start, end)),
        }
    }

    let played_percent = played_percent.clamp(0.0, 100.0);
    if let Some(active) = visual.iter().position(|(start, end)| {
        *end >= played_percent && *start <= played_percent + PLAYHEAD_SNAP_PERCENT
    }) && visual[active].0 > played_percent
    {
        visual[active].0 = played_percent;
    }
    visual
}

/// Tabular figures, so a ticking second stops shuffling the minutes sideways.
pub fn tabular() -> FontFeatures {
    static FEATURES: OnceLock<FontFeatures> = OnceLock::new();
    FEATURES
        .get_or_init(|| FontFeatures(Arc::new(vec![("tnum".into(), 1)])))
        .clone()
}

/// What the right-hand clock reads: the length, or how much of it is left.
pub fn trailing(position_ms: u64, total_ms: Option<u64>, remaining: bool) -> SharedString {
    let Some(total) = total_ms else {
        return SharedString::default();
    };
    if remaining {
        SharedString::from(format!("-{}", duration(total.saturating_sub(position_ms))))
    } else {
        duration(total)
    }
}

/// How wide both clocks are drawn, so neither resizes as the other one ticks.
pub fn clock_width(total_ms: Option<u64>) -> Rems {
    match total_ms {
        Some(total) if total >= HOUR_MS => rems(LONG),
        _ => rems(SHORT),
    }
}

/// The seek track over a slider state the caller owns, with the time under the pointer floating above it.
#[derive(IntoElement)]
pub struct SeekBar {
    id: SharedString,
    state: Entity<SliderState>,
    played: f32,
    total_ms: Option<u64>,
    ranges: Vec<BufferedRange>,
    loading: bool,
    seekable: bool,
    scrubbing: bool,
    preview: bool,
    scale: f32,
}

impl SeekBar {
    pub fn new(id: impl Into<SharedString>, state: &Entity<SliderState>) -> Self {
        Self {
            id: id.into(),
            state: state.clone(),
            played: 0.,
            total_ms: None,
            ranges: Vec::new(),
            loading: false,
            seekable: true,
            scrubbing: false,
            preview: false,
            scale: 1.,
        }
    }

    /// How far along the track is, as a 0..1 fraction.
    pub fn played(mut self, played: f32) -> Self {
        self.played = played;
        self
    }

    /// The track's length, which the preview time and whether it can be sought at all hang on.
    pub fn total(mut self, total_ms: Option<u64>) -> Self {
        self.total_ms = total_ms;
        self
    }

    pub fn buffered(mut self, ranges: &[BufferedRange]) -> Self {
        self.ranges = ranges.to_vec();
        self
    }

    /// The source is still opening the track: nothing to seek yet, and a sweep says it is alive.
    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    pub fn seekable(mut self, seekable: bool) -> Self {
        self.seekable = seekable;
        self
    }

    /// The pointer is holding the bar, which keeps the knob out.
    pub fn scrubbing(mut self, scrubbing: bool) -> Self {
        self.scrubbing = scrubbing;
        self
    }

    /// Every measure multiplied by `scale`, for a bar the person has made larger or smaller.
    pub fn scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }

    /// Float the time under the pointer while it hovers the bar.
    pub fn preview(mut self, preview: bool) -> Self {
        self.preview = preview;
        self
    }
}

impl RenderOnce for SeekBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let live = self.total_ms.is_some() && !self.loading && self.seekable;
        let scale = self.scale;
        let bars = visual_ranges(&self.ranges, self.played * 100.);
        let scanning = self.loading && bars.is_empty();
        let pointer = window.use_keyed_state(motion::channel(&self.id, PREVIEW), cx, |_, _| {
            None::<(f32, Pixels)>
        });
        let hovering = if self.preview {
            *pointer.read(cx)
        } else {
            None
        };
        let held = self.scrubbing || hovering.is_some();
        let state = self.state.clone();
        let (moved, left) = (pointer.clone(), pointer.clone());

        div()
            .id(self.id.clone())
            .relative()
            .w_full()
            .h(ThinSlider::height(HIT * scale))
            .when(live && self.preview, |hit| {
                hit.on_mouse_move(move |event: &MouseMoveEvent, _, cx| {
                    let bounds = state.read(cx).bounds();
                    if bounds.size.width <= px(0.) {
                        return;
                    }
                    let local = (event.position.x - bounds.left()).clamp(px(0.), bounds.size.width);
                    let next = Some((f32::from(local) / f32::from(bounds.size.width), local));
                    moved.update(cx, |preview, cx| {
                        if *preview != next {
                            *preview = next;
                            cx.notify();
                        }
                    });
                })
                .on_hover(move |hovered: &bool, _, cx| {
                    if !*hovered {
                        left.update(cx, |preview, cx| {
                            if preview.take().is_some() {
                                cx.notify();
                            }
                        });
                    }
                })
            })
            .child(
                ThinSlider::new(self.id.clone(), &self.state)
                    .filled(self.played)
                    .metrics(HIT * scale, BAR * scale, KNOB * scale)
                    .knob(held && live)
                    .disabled(!live)
                    .buffered(bars)
                    .scanning(scanning),
            )
            .children(hovering.zip(self.total_ms).map(|((fraction, x), total)| {
                preview_label(
                    duration((f64::from(fraction) * total as f64) as u64),
                    x,
                    scale,
                    cx,
                )
            }))
    }
}

fn preview_label(at: SharedString, x: Pixels, scale: f32, cx: &App) -> impl IntoElement {
    div()
        .absolute()
        .bottom(PREVIEW_LIFT * scale)
        .left(x)
        .ml(-PREVIEW_WIDTH * scale / 2.)
        .w(PREVIEW_WIDTH * scale)
        .text_center()
        .text_size(style::text_10() * scale)
        .font_features(tabular())
        .px_1()
        .py_0p5()
        .rounded(style::radius(cx))
        .bg(cx.theme().popover)
        .text_color(cx.theme().popover_foreground)
        .shadow_md()
        .child(at)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Elapsed,
    Trailing,
    Summary,
}

/// A clock in a box sized for the longest reading it will hold.
#[derive(IntoElement)]
pub struct TrackClock {
    id: SharedString,
    position_ms: u64,
    total_ms: Option<u64>,
    side: Side,
    toggle: bool,
    tint: Option<Hsla>,
    scale: f32,
}

impl TrackClock {
    /// Where the track is, the left-hand clock.
    pub fn new(position_ms: u64, total_ms: Option<u64>) -> Self {
        Self {
            id: "track-clock".into(),
            position_ms,
            total_ms,
            side: Side::Elapsed,
            toggle: false,
            tint: None,
            scale: 1.,
        }
    }

    /// Keys the toggle's state, for a clock sharing a parent with another.
    pub fn id(mut self, id: impl Into<SharedString>) -> Self {
        self.id = id.into();
        self
    }

    /// The right-hand clock: the length, or what is left of it.
    pub fn trailing(mut self) -> Self {
        self.side = Side::Trailing;
        self
    }

    /// Both readings in one line, as the mini surface has room for.
    pub fn summary(mut self) -> Self {
        self.side = Side::Summary;
        self
    }

    /// A click flips the right-hand clock between the length and what is left.
    pub fn remaining_toggle(mut self, toggle: bool) -> Self {
        self.toggle = toggle;
        self
    }

    /// The reading and its box multiplied by `scale`, beside a bar drawn the same way.
    pub fn scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }

    pub fn tint(mut self, tint: Hsla) -> Self {
        self.tint = Some(tint);
        self
    }
}

impl RenderOnce for TrackClock {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let remaining =
            window.use_keyed_state(motion::channel(&self.id, REMAINING), cx, |_, _| false);
        let (position, total) = (self.position_ms, self.total_ms);
        let reading = match self.side {
            Side::Elapsed => duration(position),
            Side::Trailing => trailing(position, total, *remaining.read(cx)),
            Side::Summary => match total {
                Some(_) => {
                    format!("{} {}", duration(position), trailing(position, total, true)).into()
                }
                None => duration(position),
            },
        };
        let summary = self.side == Side::Summary;
        let scale = self.scale;
        let tint = self.tint.unwrap_or(match summary {
            true => style::white(0.35, cx),
            false => style::slate_500(cx),
        });
        let clock = div()
            .id(motion::channel(&self.id, REMAINING))
            .text_size(if summary {
                style::text_9() * scale
            } else {
                style::text_10() * scale
            })
            .font_features(tabular())
            .text_color(tint)
            .when(!summary, |clock| {
                clock.w(rems(clock_width(total).0 * scale)).flex_shrink_0()
            })
            .when(self.side == Side::Elapsed, |clock| clock.text_right())
            .when(self.side == Side::Trailing && self.toggle, |clock| {
                clock.cursor_pointer().on_click(move |_, _, cx| {
                    remaining.update(cx, |remaining, cx| {
                        *remaining = !*remaining;
                        cx.notify();
                    })
                })
            });
        clock.child(reading)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(start: u64, end: u64) -> BufferedRange {
        BufferedRange {
            start,
            end,
            total: Some(1_000),
        }
    }

    #[test]
    fn small_gaps_coalesce_and_the_bar_attaches_to_the_playhead() {
        let ranges = [range(680, 700), range(704, 720)];
        assert_eq!(visual_ranges(&ranges, 65.0), vec![(65.0, 72.0)]);
    }

    #[test]
    fn distant_seek_ranges_stay_separate() {
        let ranges = [range(100, 200), range(800, 900)];
        assert_eq!(
            visual_ranges(&ranges, 10.0),
            vec![(10.0, 20.0), (80.0, 90.0)]
        );
    }

    #[test]
    fn a_range_without_a_total_is_not_drawn() {
        let ranges = [BufferedRange {
            start: 0,
            end: 10,
            total: None,
        }];
        assert!(visual_ranges(&ranges, 0.0).is_empty());
    }

    #[test]
    fn the_trailing_clock_counts_down_or_names_the_length() {
        assert_eq!(trailing(60_000, Some(180_000), true), "-2:00");
        assert_eq!(trailing(60_000, Some(180_000), false), "3:00");
        assert_eq!(trailing(60_000, None, true), "");
    }
}
