//! Material Design 3 Progress Indicator components.
//!
//! Progress indicators inform users about the status of ongoing processes,
//! such as loading an app, submitting a form, or saving updates. MD3 defines
//! two types:
//!
//! - **Linear progress indicator** — a horizontal bar that fills from
//!   leading to trailing edge
//! - **Circular progress indicator** — a circular arc that fills clockwise
//!
//! Both types support **determinate** (known progress 0.0–1.0) and
//! **indeterminate** (unknown duration) modes.
//!
//! ## Architecture
//!
//! All indicator structs use a **builder pattern** and implement `IntoElement`,
//! making them composable with GPUI's standard `.child(...)` API.
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use gpui_mobile::components::material::progress_indicator::{
//!     LinearProgressIndicator, CircularProgressIndicator,
//! };
//! use gpui_mobile::components::material::MaterialTheme;
//!
//! let theme = MaterialTheme::dark();
//!
//! // Determinate linear progress (60% complete)
//! let linear = LinearProgressIndicator::new(theme)
//!     .progress(0.6);
//!
//! // Indeterminate linear progress
//! let linear_ind = LinearProgressIndicator::new(theme)
//!     .indeterminate();
//!
//! // Determinate circular progress (45% complete)
//! let circular = CircularProgressIndicator::new(theme)
//!     .progress(0.45);
//!
//! // Indeterminate circular progress
//! let circular_ind = CircularProgressIndicator::new(theme)
//!     .indeterminate();
//! ```
//!
//! ## MD3 Specification Reference
//!
//! - Linear: 4dp height, full width, primary color active track,
//!   surface-container-highest inactive track, round caps on active track
//! - Circular: 4dp stroke width, 48dp diameter, primary color active arc,
//!   surface-container-highest inactive arc (determinate only)
//! - Indeterminate mode uses a looping animation effect; since GPUI does
//!   not yet provide built-in keyframe animations, we simulate the visual
//!   appearance using a static "pulsing" representation that looks correct
//!   at any single frame and can be animated externally via
//!   `window.request_animation_frame()`.

use gpui::{div, prelude::*, px, Hsla};

use super::theme::{color, MaterialTheme};

// ── Constants ────────────────────────────────────────────────────────────────

/// Linear indicator track height in dp (MD3 spec: 4dp).
const LINEAR_TRACK_HEIGHT: f32 = 4.0;

/// Linear indicator corner radius (MD3: fully rounded caps).
const _LINEAR_TRACK_RADIUS: f32 = 2.0;

/// Circular indicator default diameter in dp (MD3 spec: 48dp).
const CIRCULAR_DIAMETER: f32 = 48.0;

/// Circular indicator stroke width in dp (MD3 spec: 4dp).
const CIRCULAR_STROKE_WIDTH: f32 = 4.0;

/// The fraction of the track that the indeterminate "thumb" occupies
/// for the linear indicator. MD3 uses an animated sweep; we use a
/// static fraction that looks reasonable when rendered at rest.
const INDETERMINATE_LINEAR_FRACTION: f32 = 0.35;

/// The fraction of the arc that the indeterminate circular indicator
/// shows. MD3 animates from ~10% to ~70% sweep; we use a middle value.
const INDETERMINATE_CIRCULAR_FRACTION: f32 = 0.30;

/// Number of discrete segments used to approximate the circular arc.
/// More segments produce a smoother curve at the cost of more child elements.
const CIRCULAR_SEGMENTS: usize = 36;

// ═══════════════════════════════════════════════════════════════════════════════
//  Linear Progress Indicator
// ═══════════════════════════════════════════════════════════════════════════════

/// A Material Design 3 **linear progress indicator**.
///
/// Linear indicators display progress by filling a horizontal track from
/// leading to trailing edge.
///
/// # Determinate vs Indeterminate
///
/// - **Determinate**: set a value via [`.progress(0.0..=1.0)`](Self::progress).
///   The active track fills to the corresponding percentage.
/// - **Indeterminate**: call [`.indeterminate()`](Self::indeterminate). The
///   indicator shows a partial fill to suggest ongoing activity.
///
/// # Customisation
///
/// | Method | Description |
/// |--------|-------------|
/// | `.progress(f32)` | Set the fill fraction (0.0–1.0) |
/// | `.indeterminate()` | Switch to indeterminate mode |
/// | `.track_height(f32)` | Override the track height (default 4dp) |
/// | `.color(u32)` | Override the active indicator colour |
/// | `.track_color(u32)` | Override the inactive track colour |
/// | `.rounded(bool)` | Enable/disable rounded caps (default true) |
///
/// # Example
///
/// ```rust,ignore
/// let bar = LinearProgressIndicator::new(theme)
///     .progress(0.75)
///     .track_height(6.0);
/// ```
pub struct LinearProgressIndicator {
    theme: MaterialTheme,
    /// Progress value 0.0..=1.0, or `None` for indeterminate.
    value: Option<f32>,
    /// Override for the active track colour.
    active_color: Option<u32>,
    /// Override for the inactive track colour.
    track_color: Option<u32>,
    /// Track height in dp.
    height: f32,
    /// Whether to use rounded caps.
    rounded: bool,
    /// Custom element ID.
    id: Option<gpui::ElementId>,
    /// Elapsed time in seconds — drives indeterminate animation.
    time: f32,
}

impl LinearProgressIndicator {
    /// Create a new linear progress indicator with the given theme.
    ///
    /// By default it is indeterminate. Call `.progress(value)` to make it
    /// determinate.
    pub fn new(theme: MaterialTheme) -> Self {
        Self {
            theme,
            value: None,
            active_color: None,
            track_color: None,
            height: LINEAR_TRACK_HEIGHT,
            rounded: true,
            id: None,
            time: 0.0,
        }
    }

    /// Set the progress value (0.0–1.0). Switches to determinate mode.
    ///
    /// Values are clamped to the 0.0–1.0 range.
    pub fn progress(mut self, value: f32) -> Self {
        self.value = Some(value.clamp(0.0, 1.0));
        self
    }

    /// Switch to indeterminate mode (no known progress value).
    pub fn indeterminate(mut self) -> Self {
        self.value = None;
        self
    }

    /// Override the active indicator colour (default: theme primary).
    pub fn color(mut self, hex: u32) -> Self {
        self.active_color = Some(hex);
        self
    }

    /// Override the inactive track colour (default: theme surface-container-highest).
    pub fn track_color(mut self, hex: u32) -> Self {
        self.track_color = Some(hex);
        self
    }

    /// Set the track height in dp (default: 4dp).
    pub fn track_height(mut self, height: f32) -> Self {
        self.height = height.max(1.0);
        self
    }

    /// Enable or disable rounded caps on the active track (default: true).
    pub fn rounded(mut self, rounded: bool) -> Self {
        self.rounded = rounded;
        self
    }

    /// Set a custom element ID.
    pub fn id(mut self, id: impl Into<gpui::ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Set the elapsed time in seconds (drives indeterminate animation).
    pub fn time(mut self, time: f32) -> Self {
        self.time = time;
        self
    }
}

impl IntoElement for LinearProgressIndicator {
    type Element = <gpui::Div as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let t = self.theme;

        let active = color(self.active_color.unwrap_or(t.primary));
        let track_bg = color(self.track_color.unwrap_or(t.surface_container_highest));
        let radius = if self.rounded { self.height / 2.0 } else { 0.0 };

        // We use a simple segment-based approach: render the track as a
        // flex row of 100 equal flex-grow children. Each child is either
        // coloured (active) or transparent (inactive track background is
        // provided by the container). This gives us accurate percentage-
        // based fills without needing percentage widths.
        let total_segments: usize = 100;

        let (filled_start, filled_count) = match self.value {
            // Determinate: fill from the leading edge.
            Some(progress) => {
                let count = (progress * total_segments as f32).round() as usize;
                (0, count.min(total_segments))
            }
            // Indeterminate: animated partial fill that sweeps back and forth.
            None => {
                let count =
                    (INDETERMINATE_LINEAR_FRACTION * total_segments as f32).round() as usize;
                // Use a sine wave to sweep the fill position across the track.
                // Period ~2 seconds, oscillates between 0 and (total - count).
                let max_start = total_segments.saturating_sub(count);
                let phase = (self.time * std::f32::consts::PI).sin() * 0.5 + 0.5; // 0..1
                let start = (phase * max_start as f32).round() as usize;
                (start, count.min(total_segments - start))
            }
        };

        let mut track = div()
            .w_full()
            .h(px(self.height))
            .rounded(px(radius))
            .bg(track_bg)
            .overflow_hidden()
            .flex()
            .flex_row();

        for i in 0..total_segments {
            let is_filled = i >= filled_start && i < filled_start + filled_count;
            let segment = if is_filled {
                div().h_full().bg(active).flex_grow(1.)
            } else {
                div().h_full().flex_grow(1.)
            };
            track = track.child(segment);
        }

        track.into_element()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Circular Progress Indicator
// ═══════════════════════════════════════════════════════════════════════════════

/// A Material Design 3 **circular progress indicator**.
///
/// Circular indicators display progress as an arc that fills clockwise
/// from the top (12 o'clock position).
///
/// # Determinate vs Indeterminate
///
/// - **Determinate**: set a value via [`.progress(0.0..=1.0)`](Self::progress).
///   The arc fills to the corresponding percentage.
/// - **Indeterminate**: call [`.indeterminate()`](Self::indeterminate). The
///   indicator shows a partial arc to suggest ongoing activity.
///
/// # Rendering Approach
///
/// Since GPUI does not have a native arc/path drawing primitive available
/// from the `div()` builder API, we approximate the circular indicator
/// using a ring of small dot segments arranged in a circle. Each segment
/// is either coloured (active) or transparent (inactive), producing a
/// visually convincing circular progress arc.
///
/// # Customisation
///
/// | Method | Description |
/// |--------|-------------|
/// | `.progress(f32)` | Set the fill fraction (0.0–1.0) |
/// | `.indeterminate()` | Switch to indeterminate mode |
/// | `.diameter(f32)` | Override the overall diameter (default 48dp) |
/// | `.stroke_width(f32)` | Override the arc stroke width (default 4dp) |
/// | `.color(u32)` | Override the active arc colour |
/// | `.track_color(u32)` | Override the inactive arc colour |
///
/// # Example
///
/// ```rust,ignore
/// let spinner = CircularProgressIndicator::new(theme)
///     .progress(0.45)
///     .diameter(64.0);
/// ```
pub struct CircularProgressIndicator {
    theme: MaterialTheme,
    /// Progress value 0.0..=1.0, or `None` for indeterminate.
    value: Option<f32>,
    /// Override for the active arc colour.
    active_color: Option<u32>,
    /// Override for the inactive arc/track colour.
    track_color: Option<u32>,
    /// Overall diameter in dp.
    diameter: f32,
    /// Arc stroke width in dp.
    stroke_width: f32,
    /// Custom element ID.
    id: Option<gpui::ElementId>,
    /// Elapsed time in seconds — drives indeterminate animation.
    time: f32,
}

impl CircularProgressIndicator {
    /// Create a new circular progress indicator with the given theme.
    ///
    /// By default it is indeterminate. Call `.progress(value)` to make it
    /// determinate.
    pub fn new(theme: MaterialTheme) -> Self {
        Self {
            theme,
            value: None,
            active_color: None,
            track_color: None,
            diameter: CIRCULAR_DIAMETER,
            stroke_width: CIRCULAR_STROKE_WIDTH,
            id: None,
            time: 0.0,
        }
    }

    /// Set the progress value (0.0–1.0). Switches to determinate mode.
    ///
    /// Values are clamped to the 0.0–1.0 range.
    pub fn progress(mut self, value: f32) -> Self {
        self.value = Some(value.clamp(0.0, 1.0));
        self
    }

    /// Switch to indeterminate mode (no known progress value).
    pub fn indeterminate(mut self) -> Self {
        self.value = None;
        self
    }

    /// Override the active arc colour (default: theme primary).
    pub fn color(mut self, hex: u32) -> Self {
        self.active_color = Some(hex);
        self
    }

    /// Override the inactive track colour (default: theme surface-container-highest).
    ///
    /// Note: in indeterminate mode, the track is typically not shown
    /// (transparent), following the MD3 spec. In determinate mode the
    /// full ring is shown as the inactive track.
    pub fn track_color(mut self, hex: u32) -> Self {
        self.track_color = Some(hex);
        self
    }

    /// Set the overall diameter in dp (default: 48dp).
    pub fn diameter(mut self, diameter: f32) -> Self {
        self.diameter = diameter.max(8.0);
        self
    }

    /// Set the arc stroke width in dp (default: 4dp).
    pub fn stroke_width(mut self, width: f32) -> Self {
        self.stroke_width = width.max(1.0);
        self
    }

    /// Set a custom element ID.
    pub fn id(mut self, id: impl Into<gpui::ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Set the elapsed time in seconds (drives indeterminate animation).
    pub fn time(mut self, time: f32) -> Self {
        self.time = time;
        self
    }
}

impl IntoElement for CircularProgressIndicator {
    type Element = <gpui::Div as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let t = self.theme;

        let active: Hsla = color(self.active_color.unwrap_or(t.primary));
        let track_bg: Hsla = color(self.track_color.unwrap_or(t.surface_container_highest));
        let transparent: Hsla = gpui::hsla(0.0, 0.0, 0.0, 0.0);

        let diameter = self.diameter;
        let stroke = self.stroke_width;
        let radius = diameter / 2.0;
        let dot_size = stroke;

        // Determine which segments are active.
        let total = CIRCULAR_SEGMENTS;
        let (active_start, active_count, show_track) = match self.value {
            Some(progress) => {
                let count = (progress * total as f32).round() as usize;
                (0, count, true) // determinate: start at top, show inactive track
            }
            None => {
                // Indeterminate: a partial arc that rotates continuously.
                let count = (INDETERMINATE_CIRCULAR_FRACTION * total as f32).round() as usize;
                // Rotate based on time — one full rotation per ~1.5 seconds.
                let rotation = (self.time * 0.67 * total as f32) as usize % total;
                (rotation, count, false) // indeterminate: no inactive track
            }
        };

        // Container: a square box of `diameter × diameter`, with dots
        // positioned absolutely in a circle.
        let mut container = div()
            .relative()
            .w(px(diameter))
            .h(px(diameter))
            .flex_shrink_0();

        // Place dots around the circle.
        for i in 0..total {
            // Angle: start at 12 o'clock (top center), go clockwise.
            // 0 radians = top = -π/2 in standard math orientation.
            let angle = -std::f32::consts::FRAC_PI_2
                + (i as f32 / total as f32) * 2.0 * std::f32::consts::PI;

            // Position of the dot center.
            let cx_pos = radius + (radius - stroke / 2.0) * angle.cos() - dot_size / 2.0;
            let cy_pos = radius + (radius - stroke / 2.0) * angle.sin() - dot_size / 2.0;

            // Determine if this segment is active.
            let is_active = if active_count == 0 {
                false
            } else {
                let normalized = (i + total - active_start) % total;
                normalized < active_count
            };

            let dot_color = if is_active {
                active
            } else if show_track {
                track_bg
            } else {
                transparent
            };

            let dot = div()
                .absolute()
                .left(px(cx_pos))
                .top(px(cy_pos))
                .w(px(dot_size))
                .h(px(dot_size))
                .rounded(px(dot_size / 2.0))
                .bg(dot_color);

            container = container.child(dot);
        }

        container.into_element()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Demo / Showcase
// ═══════════════════════════════════════════════════════════════════════════════

/// Render a static demo of all progress indicator variants.
///
/// Used in the component showcase gallery to display both linear and
/// circular indicators in determinate and indeterminate states.
/// Render a demo of all progress indicator variants.
///
/// Pass a monotonically increasing `time` (in seconds) to animate
/// indeterminate indicators. Use `0.0` for a static snapshot.
pub fn progress_indicator_demo_animated(dark: bool, time: f32) -> impl IntoElement {
    progress_indicator_demo_inner(dark, time)
}

/// Render a static demo of all progress indicator variants (backwards-compat).
pub fn progress_indicator_demo(dark: bool) -> impl IntoElement {
    progress_indicator_demo_inner(dark, 0.0)
}

fn progress_indicator_demo_inner(dark: bool, time: f32) -> impl IntoElement {
    let theme = MaterialTheme::from_appearance(dark);
    let label_color = color(theme.on_surface_variant);
    let heading_color = color(theme.on_surface);

    div()
        .flex()
        .flex_col()
        .gap_6()
        .w_full()
        .p_4()
        // ── Section: Linear Indicators ───────────────────────────────────
        .child(
            div()
                .flex()
                .flex_col()
                .gap_4()
                .child(
                    div()
                        .text_base()
                        .text_color(heading_color)
                        .child("Linear Progress Indicators"),
                )
                // Determinate — 0%
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(label_color)
                                .child("Determinate — 0%"),
                        )
                        .child(LinearProgressIndicator::new(theme).progress(0.0)),
                )
                // Determinate — 25%
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(label_color)
                                .child("Determinate — 25%"),
                        )
                        .child(LinearProgressIndicator::new(theme).progress(0.25)),
                )
                // Determinate — 50%
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(label_color)
                                .child("Determinate — 50%"),
                        )
                        .child(LinearProgressIndicator::new(theme).progress(0.50)),
                )
                // Determinate — 75%
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(label_color)
                                .child("Determinate — 75%"),
                        )
                        .child(LinearProgressIndicator::new(theme).progress(0.75)),
                )
                // Determinate — 100%
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(label_color)
                                .child("Determinate — 100%"),
                        )
                        .child(LinearProgressIndicator::new(theme).progress(1.0)),
                )
                // Indeterminate
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(label_color)
                                .child("Indeterminate"),
                        )
                        .child(
                            LinearProgressIndicator::new(theme)
                                .indeterminate()
                                .time(time),
                        ),
                )
                // Custom height
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(label_color)
                                .child("Custom height (8dp) — 60%"),
                        )
                        .child(
                            LinearProgressIndicator::new(theme)
                                .progress(0.60)
                                .track_height(8.0),
                        ),
                )
                // No rounded caps
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(label_color)
                                .child("Square caps — 40%"),
                        )
                        .child(
                            LinearProgressIndicator::new(theme)
                                .progress(0.40)
                                .rounded(false),
                        ),
                ),
        )
        // ── Section: Circular Indicators ─────────────────────────────────
        .child(
            div()
                .flex()
                .flex_col()
                .gap_4()
                .child(
                    div()
                        .text_base()
                        .text_color(heading_color)
                        .child("Circular Progress Indicators"),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .gap_6()
                        .items_end()
                        // 25%
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap_2()
                                .child(CircularProgressIndicator::new(theme).progress(0.25))
                                .child(div().text_xs().text_color(label_color).child("25%")),
                        )
                        // 50%
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap_2()
                                .child(CircularProgressIndicator::new(theme).progress(0.50))
                                .child(div().text_xs().text_color(label_color).child("50%")),
                        )
                        // 75%
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap_2()
                                .child(CircularProgressIndicator::new(theme).progress(0.75))
                                .child(div().text_xs().text_color(label_color).child("75%")),
                        )
                        // 100%
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap_2()
                                .child(CircularProgressIndicator::new(theme).progress(1.0))
                                .child(div().text_xs().text_color(label_color).child("100%")),
                        )
                        // Indeterminate
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap_2()
                                .child(
                                    CircularProgressIndicator::new(theme)
                                        .indeterminate()
                                        .time(time),
                                )
                                .child(div().text_xs().text_color(label_color).child("Indet.")),
                        ),
                )
                // Large custom diameter
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_6()
                        .items_end()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap_2()
                                .child(
                                    CircularProgressIndicator::new(theme)
                                        .progress(0.65)
                                        .diameter(72.0)
                                        .stroke_width(6.0),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(label_color)
                                        .child("Large (72dp)"),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap_2()
                                .child(
                                    CircularProgressIndicator::new(theme)
                                        .progress(0.3)
                                        .diameter(32.0)
                                        .stroke_width(3.0),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(label_color)
                                        .child("Small (32dp)"),
                                ),
                        ),
                ),
        )
}
