#![allow(unused_imports)]
//! Material Design 3 Form Control components.
//!
//! This module provides interactive form controls following the MD3 specification:
//!
//! - **Checkbox** — a square toggle with check/indeterminate states
//! - **Radio** — a circular single-selection control
//! - **Switch** — a toggle switch with thumb and track
//! - **Slider** — a continuous or discrete value selector
//! - **RangeSlider** — a two-thumb range selector
//!
//! All controls follow a builder pattern and implement `IntoElement`.
//! Click/change handlers use GPUI's `on_mouse_down` signature, fully
//! compatible with `cx.listener(...)`.
//!
//! # Examples
//!
//! ```rust,ignore
//! use gpui_mobile::components::material::controls::*;
//! use gpui_mobile::components::material::theme::MaterialTheme;
//!
//! let theme = MaterialTheme::dark();
//!
//! // Checkbox
//! let cb = Checkbox::new(theme)
//!     .checked(true)
//!     .label("Accept terms")
//!     .on_toggle(cx.listener(|this, _, _, cx| {
//!         this.accepted = !this.accepted;
//!         cx.notify();
//!     }));
//!
//! // Radio
//! let radio = Radio::new(theme)
//!     .selected(self.choice == 0)
//!     .label("Option A")
//!     .on_select(cx.listener(|this, _, _, cx| {
//!         this.choice = 0;
//!         cx.notify();
//!     }));
//!
//! // Switch
//! let sw = Switch::new(theme)
//!     .on(self.dark_mode)
//!     .label("Dark mode")
//!     .on_toggle(cx.listener(|this, _, _, cx| {
//!         this.dark_mode = !this.dark_mode;
//!         cx.notify();
//!     }));
//!
//! // Slider
//! let slider = Slider::new(theme)
//!     .value(0.5)
//!     .label("Volume");
//!
//! // RangeSlider
//! let range = RangeSlider::new(theme)
//!     .range(0.2, 0.8)
//!     .label("Price range");
//! ```

use gpui::{div, prelude::*, px, rgb, Hsla, MouseButton, MouseDownEvent, Window};

use super::theme::{color, MaterialTheme, TRANSPARENT};

// ── Click handler alias ──────────────────────────────────────────────────────

type ClickHandler = Box<dyn Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static>;

// ═══════════════════════════════════════════════════════════════════════════════
//  Constants
// ═══════════════════════════════════════════════════════════════════════════════

// Checkbox
const CHECKBOX_SIZE: f32 = 18.0;
const CHECKBOX_CONTAINER_SIZE: f32 = 40.0;
const CHECKBOX_RADIUS: f32 = 2.0;
const CHECKBOX_BORDER_WIDTH: f32 = 2.0;

// Radio
const RADIO_SIZE: f32 = 20.0;
const RADIO_CONTAINER_SIZE: f32 = 40.0;
const RADIO_INNER_SIZE: f32 = 10.0;

// Switch
const SWITCH_TRACK_WIDTH: f32 = 52.0;
const SWITCH_TRACK_HEIGHT: f32 = 32.0;
const SWITCH_TRACK_RADIUS: f32 = 16.0;
const SWITCH_THUMB_SIZE_OFF: f32 = 16.0;
const SWITCH_THUMB_SIZE_ON: f32 = 24.0;
const SWITCH_THUMB_MARGIN: f32 = 4.0;

// Slider
const SLIDER_TRACK_HEIGHT: f32 = 4.0;
const SLIDER_TRACK_HEIGHT_ACTIVE: f32 = 4.0;
const SLIDER_THUMB_SIZE: f32 = 20.0;
const SLIDER_TOTAL_HEIGHT: f32 = 44.0;

// Label gap
const LABEL_GAP: f32 = 12.0;

// ═══════════════════════════════════════════════════════════════════════════════
//  CheckboxState
// ═══════════════════════════════════════════════════════════════════════════════

/// The visual state of a checkbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CheckboxState {
    /// Unchecked (empty box).
    #[default]
    Unchecked,
    /// Checked (filled box with checkmark).
    Checked,
    /// Indeterminate (filled box with a dash) — used for "select all" when
    /// only some children are selected.
    Indeterminate,
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Checkbox
// ═══════════════════════════════════════════════════════════════════════════════

/// A Material Design 3 **Checkbox**.
///
/// A square toggle control with three visual states: unchecked, checked,
/// and indeterminate. Supports an optional text label and error state.
///
/// When checked, the box is filled with the primary color and displays
/// a white checkmark. When indeterminate, it shows a horizontal dash.
///
/// # Layout
///
/// ```text
/// ┌──┐
/// │✓ │  Accept terms and conditions
/// └──┘
/// ```
pub struct Checkbox {
    theme: MaterialTheme,
    state: CheckboxState,
    label: Option<String>,
    on_toggle: Option<ClickHandler>,
    disabled: bool,
    error: bool,
    id: Option<gpui::ElementId>,
}

impl Checkbox {
    /// Create a new unchecked checkbox.
    pub fn new(theme: MaterialTheme) -> Self {
        Self {
            theme,
            state: CheckboxState::Unchecked,
            label: None,
            on_toggle: None,
            disabled: false,
            error: false,
            id: None,
        }
    }

    /// Set the checkbox to checked.
    pub fn checked(mut self, checked: bool) -> Self {
        self.state = if checked {
            CheckboxState::Checked
        } else {
            CheckboxState::Unchecked
        };
        self
    }

    /// Set the checkbox state explicitly.
    pub fn state(mut self, state: CheckboxState) -> Self {
        self.state = state;
        self
    }

    /// Set the indeterminate state.
    pub fn indeterminate(mut self) -> Self {
        self.state = CheckboxState::Indeterminate;
        self
    }

    /// Set an optional text label displayed to the right of the checkbox.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Set the toggle handler, invoked when the checkbox is tapped.
    ///
    /// The handler signature matches GPUI's `on_mouse_down`, so
    /// `cx.listener(...)` can be used directly.
    pub fn on_toggle(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_toggle = Some(Box::new(handler));
        self
    }

    /// Mark the checkbox as disabled (non-interactive, muted colors).
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Mark the checkbox as in an error state (red border/fill).
    pub fn error(mut self, error: bool) -> Self {
        self.error = error;
        self
    }

    /// Set an explicit element ID.
    pub fn id(mut self, id: impl Into<gpui::ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }
}

impl IntoElement for Checkbox {
    type Element = <gpui::Stateful<gpui::Div> as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let t = self.theme;
        let is_filled = self.state != CheckboxState::Unchecked;

        // Determine colors based on state (all Hsla for type consistency)
        let (box_bg, box_border, check_color): (Hsla, Hsla, Hsla) = if self.disabled {
            if is_filled {
                (
                    gpui::hsla(0.0, 0.0, 0.5, 0.38),
                    TRANSPARENT,
                    color(t.surface),
                )
            } else {
                (TRANSPARENT, gpui::hsla(0.0, 0.0, 0.5, 0.38), TRANSPARENT)
            }
        } else if self.error {
            if is_filled {
                (color(t.error), color(t.error), color(t.on_error))
            } else {
                (TRANSPARENT, color(t.error), TRANSPARENT)
            }
        } else if is_filled {
            (color(t.primary), color(t.primary), color(t.on_primary))
        } else {
            (TRANSPARENT, color(t.on_surface_variant), TRANSPARENT)
        };

        let check_mark = match self.state {
            CheckboxState::Checked => "✓",
            CheckboxState::Indeterminate => "–",
            CheckboxState::Unchecked => "",
        };

        let elem_id = self
            .id
            .unwrap_or_else(|| gpui::ElementId::Name("checkbox".into()));

        // The checkbox box itself
        let checkbox_box = div()
            .flex()
            .items_center()
            .justify_center()
            .size(px(CHECKBOX_SIZE))
            .rounded(px(CHECKBOX_RADIUS))
            .bg(box_bg)
            .border(px(CHECKBOX_BORDER_WIDTH))
            .border_color(box_border)
            .text_size(px(14.0))
            .line_height(px(CHECKBOX_SIZE))
            .text_color(check_color)
            .child(check_mark);

        // Touch target container (centered around the box)
        let touch_target = div()
            .flex()
            .items_center()
            .justify_center()
            .size(px(CHECKBOX_CONTAINER_SIZE))
            .rounded_full()
            .child(checkbox_box);

        // Row with optional label
        let mut row = div()
            .id(elem_id)
            .flex()
            .flex_row()
            .items_center()
            .cursor_pointer();

        row = row.child(touch_target);

        if let Some(label_text) = self.label {
            let label_color: Hsla = if self.disabled {
                gpui::hsla(0.0, 0.0, 0.5, 0.38)
            } else if self.error {
                color(t.error)
            } else {
                color(t.on_surface)
            };
            row = row.child(div().text_sm().text_color(label_color).child(label_text));
        }

        if let Some(handler) = self.on_toggle {
            if !self.disabled {
                row = row.on_mouse_down(MouseButton::Left, handler);
            }
        }

        row.into_element()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Radio
// ═══════════════════════════════════════════════════════════════════════════════

/// A Material Design 3 **Radio Button**.
///
/// A circular single-selection control. When selected, displays a filled
/// inner circle within the outer ring. Radio buttons should be used in
/// groups where only one option can be active at a time.
///
/// # Layout
///
/// ```text
/// (●)  Option A       ← selected
/// ( )  Option B       ← unselected
/// ( )  Option C
/// ```
pub struct Radio {
    theme: MaterialTheme,
    selected: bool,
    label: Option<String>,
    on_select: Option<ClickHandler>,
    disabled: bool,
    id: Option<gpui::ElementId>,
}

impl Radio {
    /// Create a new unselected radio button.
    pub fn new(theme: MaterialTheme) -> Self {
        Self {
            theme,
            selected: false,
            label: None,
            on_select: None,
            disabled: false,
            id: None,
        }
    }

    /// Set whether this radio button is selected.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Set an optional text label displayed to the right.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Set the selection handler, invoked when this radio is tapped.
    pub fn on_select(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_select = Some(Box::new(handler));
        self
    }

    /// Mark as disabled.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Set an explicit element ID.
    pub fn id(mut self, id: impl Into<gpui::ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }
}

impl IntoElement for Radio {
    type Element = <gpui::Stateful<gpui::Div> as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let t = self.theme;

        // Determine colors (all Hsla for type consistency)
        let (ring_color, inner_color): (Hsla, Hsla) = if self.disabled {
            (
                gpui::hsla(0.0, 0.0, 0.5, 0.38),
                if self.selected {
                    gpui::hsla(0.0, 0.0, 0.5, 0.38)
                } else {
                    TRANSPARENT
                },
            )
        } else if self.selected {
            (color(t.primary), color(t.primary))
        } else {
            (color(t.on_surface_variant), TRANSPARENT)
        };

        let elem_id = self
            .id
            .unwrap_or_else(|| gpui::ElementId::Name("radio".into()));

        // Outer ring
        let mut radio_circle = div()
            .flex()
            .items_center()
            .justify_center()
            .size(px(RADIO_SIZE))
            .rounded_full()
            .border(px(2.0))
            .border_color(ring_color);

        // Inner filled circle (only when selected)
        if self.selected {
            radio_circle = radio_circle.child(
                div()
                    .size(px(RADIO_INNER_SIZE))
                    .rounded_full()
                    .bg(inner_color),
            );
        }

        // Touch target container
        let touch_target = div()
            .flex()
            .items_center()
            .justify_center()
            .size(px(RADIO_CONTAINER_SIZE))
            .rounded_full()
            .child(radio_circle);

        // Row with optional label
        let mut row = div()
            .id(elem_id)
            .flex()
            .flex_row()
            .items_center()
            .cursor_pointer();

        row = row.child(touch_target);

        if let Some(label_text) = self.label {
            let label_color: Hsla = if self.disabled {
                gpui::hsla(0.0, 0.0, 0.5, 0.38)
            } else {
                color(t.on_surface)
            };
            row = row.child(div().text_sm().text_color(label_color).child(label_text));
        }

        if let Some(handler) = self.on_select {
            if !self.disabled {
                row = row.on_mouse_down(MouseButton::Left, handler);
            }
        }

        row.into_element()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Switch
// ═══════════════════════════════════════════════════════════════════════════════

/// A Material Design 3 **Switch**.
///
/// A toggle control with a thumb that slides along a track. When "on",
/// the track is filled with the primary color and the thumb is enlarged.
/// When "off", the track is outlined and the thumb is smaller.
///
/// Optionally displays icons inside the thumb (e.g. "✓" when on,
/// "✕" when off) and a text label.
///
/// # Layout
///
/// ```text
/// ┌───────────────────┐
/// │     ┌────┐        │  Dark mode
/// │     │ ✓  │  ●━━━━ │  ← on state
/// │     └────┘        │
/// └───────────────────┘
/// ```
pub struct Switch {
    theme: MaterialTheme,
    is_on: bool,
    label: Option<String>,
    on_toggle: Option<ClickHandler>,
    disabled: bool,
    /// Icon displayed in the thumb when "on".
    icon_on: Option<String>,
    /// Icon displayed in the thumb when "off".
    icon_off: Option<String>,
    id: Option<gpui::ElementId>,
}

impl Switch {
    /// Create a new switch in the "off" state.
    pub fn new(theme: MaterialTheme) -> Self {
        Self {
            theme,
            is_on: false,
            label: None,
            on_toggle: None,
            disabled: false,
            icon_on: None,
            icon_off: None,
            id: None,
        }
    }

    /// Set whether the switch is on.
    pub fn on(mut self, is_on: bool) -> Self {
        self.is_on = is_on;
        self
    }

    /// Set an optional text label displayed to the right of the switch.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Set the toggle handler, invoked when the switch is tapped.
    pub fn on_toggle(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_toggle = Some(Box::new(handler));
        self
    }

    /// Mark as disabled.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Show icons in the thumb.
    ///
    /// Typically "✓" for on and "✕" for off per the MD3 spec.
    pub fn with_icons(mut self) -> Self {
        self.icon_on = Some("✓".into());
        self.icon_off = Some("✕".into());
        self
    }

    /// Set custom icons for the on and off states.
    pub fn icons(mut self, on_icon: impl Into<String>, off_icon: impl Into<String>) -> Self {
        self.icon_on = Some(on_icon.into());
        self.icon_off = Some(off_icon.into());
        self
    }

    /// Set an explicit element ID.
    pub fn id(mut self, id: impl Into<gpui::ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }
}

impl IntoElement for Switch {
    type Element = <gpui::Stateful<gpui::Div> as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let t = self.theme;
        let has_icon = self.icon_on.is_some() || self.icon_off.is_some();

        // Track colors (all Hsla for type consistency)
        let (track_bg, track_border): (Hsla, Hsla) = if self.disabled {
            if self.is_on {
                (gpui::hsla(0.0, 0.0, 0.5, 0.12), TRANSPARENT)
            } else {
                (
                    gpui::hsla(0.0, 0.0, 0.5, 0.12),
                    gpui::hsla(0.0, 0.0, 0.5, 0.12),
                )
            }
        } else if self.is_on {
            (color(t.primary), color(t.primary))
        } else {
            (color(t.surface_container_highest), color(t.outline))
        };

        // Thumb colors (all Hsla for type consistency)
        let (thumb_bg, thumb_icon_color): (Hsla, Hsla) = if self.disabled {
            if self.is_on {
                (color(t.surface), gpui::hsla(0.0, 0.0, 0.5, 0.38))
            } else {
                (
                    gpui::hsla(0.0, 0.0, 0.5, 0.38),
                    gpui::hsla(0.0, 0.0, 0.5, 0.38),
                )
            }
        } else if self.is_on {
            (color(t.on_primary), color(t.on_primary_container))
        } else {
            (color(t.outline), color(t.surface_container_highest))
        };

        // Thumb size: larger when on (24dp) or when has icon, smaller when off without icon (16dp)
        let thumb_size = if self.is_on || has_icon {
            SWITCH_THUMB_SIZE_ON
        } else {
            SWITCH_THUMB_SIZE_OFF
        };

        let elem_id = self
            .id
            .unwrap_or_else(|| gpui::ElementId::Name("switch".into()));

        // Build the thumb
        let mut thumb = div()
            .flex()
            .items_center()
            .justify_center()
            .size(px(thumb_size))
            .rounded_full()
            .bg(thumb_bg);

        // Thumb icon
        if has_icon {
            let icon_text = if self.is_on {
                self.icon_on.as_deref().unwrap_or("")
            } else {
                self.icon_off.as_deref().unwrap_or("")
            };
            if !icon_text.is_empty() {
                thumb = thumb.child(
                    div()
                        .text_size(px(16.0))
                        .line_height(px(16.0))
                        .text_color(thumb_icon_color)
                        .child(icon_text.to_string()),
                );
            }
        }

        // Build the track
        let track = div()
            .flex()
            .flex_row()
            .items_center()
            .w(px(SWITCH_TRACK_WIDTH))
            .h(px(SWITCH_TRACK_HEIGHT))
            .rounded(px(SWITCH_TRACK_RADIUS))
            .bg(track_bg)
            .border(px(2.0))
            .border_color(track_border)
            // Position thumb: left when off, right when on
            .when(self.is_on, |d| d.justify_end())
            .when(!self.is_on, |d| d.justify_start())
            .px(px(SWITCH_THUMB_MARGIN))
            .child(thumb);

        // Row with optional label
        let mut row = div()
            .id(elem_id)
            .flex()
            .flex_row()
            .items_center()
            .gap(px(LABEL_GAP))
            .cursor_pointer();

        row = row.child(track);

        if let Some(label_text) = self.label {
            let label_color: Hsla = if self.disabled {
                gpui::hsla(0.0, 0.0, 0.5, 0.38)
            } else {
                color(t.on_surface)
            };
            row = row.child(div().text_sm().text_color(label_color).child(label_text));
        }

        if let Some(handler) = self.on_toggle {
            if !self.disabled {
                row = row.on_mouse_down(MouseButton::Left, handler);
            }
        }

        row.into_element()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Slider
// ═══════════════════════════════════════════════════════════════════════════════

/// A Material Design 3 **Slider**.
///
/// A continuous (or discrete) value selector that displays a track with
/// a thumb indicator. The thumb position represents the current value
/// within a range (default 0.0–1.0).
///
/// **Note:** Since GPUI does not have built-in drag gesture handling,
/// this slider is rendered as a visual representation. For interactive
/// use, wrap it in a view that handles `MouseMoveEvent` to update the
/// value based on horizontal drag position.
///
/// # Layout
///
/// ```text
/// ┌─────────────────────────────────────┐
/// │ ━━━━━━━━━━━━━━━●─────────────────── │
/// │                ↑ thumb at 40%        │
/// └─────────────────────────────────────┘
/// ```
///
/// ## Discrete steps
///
/// When `steps` is set, the slider snaps to evenly spaced positions
/// and tick marks are shown on the track.
pub struct Slider {
    theme: MaterialTheme,
    /// Current value in the range 0.0..=1.0.
    value: f32,
    /// Optional label displayed above or beside the slider.
    label: Option<String>,
    /// Number of discrete steps (0 = continuous).
    steps: u32,
    disabled: bool,
    /// Whether to show the value label tooltip above the thumb.
    show_value_label: bool,
    /// Custom minimum label (e.g. "0").
    min_label: Option<String>,
    /// Custom maximum label (e.g. "100").
    max_label: Option<String>,
    id: Option<gpui::ElementId>,
    on_change: Option<ClickHandler>,
}

impl Slider {
    /// Create a new slider with a default value of 0.0.
    pub fn new(theme: MaterialTheme) -> Self {
        Self {
            theme,
            value: 0.0,
            label: None,
            steps: 0,
            disabled: false,
            show_value_label: false,
            min_label: None,
            max_label: None,
            id: None,
            on_change: None,
        }
    }

    /// Set the current value (clamped to 0.0..=1.0).
    pub fn value(mut self, value: f32) -> Self {
        self.value = value.clamp(0.0, 1.0);
        self
    }

    /// Set an optional label.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Set the number of discrete steps (0 = continuous, the default).
    pub fn steps(mut self, steps: u32) -> Self {
        self.steps = steps;
        self
    }

    /// Mark as disabled.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Show a value label above the thumb.
    pub fn show_value_label(mut self, show: bool) -> Self {
        self.show_value_label = show;
        self
    }

    /// Set min/max labels displayed at the ends of the track.
    pub fn range_labels(
        mut self,
        min_label: impl Into<String>,
        max_label: impl Into<String>,
    ) -> Self {
        self.min_label = Some(min_label.into());
        self.max_label = Some(max_label.into());
        self
    }

    /// Set an explicit element ID.
    pub fn id(mut self, id: impl Into<gpui::ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Set a click handler (tapped position for approximate value selection).
    pub fn on_change(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_change = Some(Box::new(handler));
        self
    }
}

impl IntoElement for Slider {
    type Element = <gpui::Stateful<gpui::Div> as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let t = self.theme;

        let active_color: Hsla = if self.disabled {
            gpui::hsla(0.0, 0.0, 0.5, 0.38)
        } else {
            color(t.primary)
        };

        let inactive_color: Hsla = if self.disabled {
            gpui::hsla(0.0, 0.0, 0.5, 0.12)
        } else {
            color(t.surface_container_highest)
        };

        let thumb_color: Hsla = if self.disabled {
            gpui::hsla(0.0, 0.0, 0.5, 0.38)
        } else {
            color(t.primary)
        };

        let label_color: Hsla = if self.disabled {
            gpui::hsla(0.0, 0.0, 0.5, 0.38)
        } else {
            color(t.on_surface)
        };

        let elem_id = self
            .id
            .unwrap_or_else(|| gpui::ElementId::Name("slider".into()));

        // We simulate the slider track using two divs:
        // - Active portion (left, filled)
        // - Inactive portion (right, subdued)
        // The thumb is positioned as absolute element.

        // Percentage for layout (integer for readability)
        let pct = (self.value * 100.0).round() as i32;
        let pct_f = self.value;

        let mut container = div()
            .id(elem_id)
            .flex()
            .flex_col()
            .gap(px(4.0))
            .w_full()
            .cursor_pointer();

        // Label
        if let Some(label_text) = self.label {
            container = container.child(div().text_sm().text_color(label_color).child(label_text));
        }

        // Track row (min_label + track + max_label)
        let mut track_row = div().flex().flex_row().items_center().gap(px(8.0)).w_full();

        // Min label
        if let Some(ref min_text) = self.min_label {
            track_row = track_row.child(
                div()
                    .text_xs()
                    .text_color(rgb(t.on_surface_variant))
                    .child(min_text.clone()),
            );
        }

        // The track itself — a horizontal bar with active and inactive portions.
        // We use a relative container and position the thumb as absolute.
        let mut track_container = div()
            .flex_1()
            .h(px(SLIDER_TOTAL_HEIGHT))
            .flex()
            .flex_row()
            .items_center()
            .relative();

        // Track background (inactive — full width)
        track_container = track_container.child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .h(px(SLIDER_TRACK_HEIGHT))
                .rounded(px(SLIDER_TRACK_HEIGHT / 2.0))
                .bg(inactive_color),
        );

        // Track active portion (from left to thumb position)
        // We approximate the width using a percentage. Since GPUI doesn't have
        // percentage-based widths natively, we render the active track
        // as part of the flex layout.
        track_container = track_container.child(
            div()
                .absolute()
                .left_0()
                .h(px(SLIDER_TRACK_HEIGHT_ACTIVE))
                .rounded(px(SLIDER_TRACK_HEIGHT_ACTIVE / 2.0))
                .bg(active_color)
                // We use a proportional width. Since we can't do % in GPUI,
                // we put this as a child that extends from the left. The parent
                // container's width is dynamic, so the absolute positioning
                // provides an approximation. For the demo, we use a fixed
                // width based on a standard track size.
                .w(gpui::relative(pct_f)),
        );

        // Tick marks for discrete sliders
        if self.steps > 1 {
            let mut ticks = div()
                .absolute()
                .left_0()
                .right_0()
                .h(px(SLIDER_TRACK_HEIGHT))
                .flex()
                .flex_row()
                .items_center()
                .justify_between();

            for i in 0..=self.steps {
                let tick_active = (i as f32 / self.steps as f32) <= self.value;
                let tick_color: Hsla = if tick_active {
                    color(t.on_primary)
                } else {
                    color(t.on_surface_variant)
                };
                ticks = ticks.child(div().size(px(4.0)).rounded_full().bg(tick_color));
            }

            track_container = track_container.child(ticks);
        }

        // Thumb — rendered as a circle centered on the track.
        // We position it using left percentage.
        let mut thumb = div()
            .absolute()
            .top(px((SLIDER_TOTAL_HEIGHT - SLIDER_THUMB_SIZE) / 2.0))
            .left(gpui::relative(pct_f))
            // Offset to center the thumb on the value position
            .ml(px(-(SLIDER_THUMB_SIZE / 2.0)))
            .size(px(SLIDER_THUMB_SIZE))
            .rounded_full()
            .bg(thumb_color);

        // Value label tooltip above the thumb
        if self.show_value_label && !self.disabled {
            thumb = thumb.child(
                div()
                    .absolute()
                    .bottom(px(SLIDER_THUMB_SIZE + 4.0))
                    .left(px(-4.0))
                    .min_w(px(28.0))
                    .h(px(28.0))
                    .px(px(6.0))
                    .rounded(px(14.0))
                    .bg(rgb(t.primary))
                    .text_color(rgb(t.on_primary))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(12.0))
                    .child(format!("{}", pct)),
            );
        }

        track_container = track_container.child(thumb);

        track_row = track_row.child(track_container);

        // Max label
        if let Some(ref max_text) = self.max_label {
            track_row = track_row.child(
                div()
                    .text_xs()
                    .text_color(rgb(t.on_surface_variant))
                    .child(max_text.clone()),
            );
        }

        container = container.child(track_row);

        // Click handler
        if let Some(handler) = self.on_change {
            if !self.disabled {
                container = container.on_mouse_down(MouseButton::Left, handler);
            }
        }

        container.into_element()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  RangeSlider
// ═══════════════════════════════════════════════════════════════════════════════

/// A Material Design 3 **Range Slider**.
///
/// A two-thumb slider for selecting a range of values. The active track
/// portion extends between the two thumbs.
///
/// Like the single-thumb [`Slider`], this component provides a visual
/// representation. Interactive drag handling requires wrapping in a view
/// that processes `MouseMoveEvent`.
///
/// # Layout
///
/// ```text
/// ┌─────────────────────────────────────┐
/// │ ──────●━━━━━━━━━━━━●─────────────── │
/// │       ↑ start      ↑ end            │
/// └─────────────────────────────────────┘
/// ```
pub struct RangeSlider {
    theme: MaterialTheme,
    /// Start value in the range 0.0..=1.0.
    start: f32,
    /// End value in the range 0.0..=1.0.
    end: f32,
    /// Optional label displayed above the slider.
    label: Option<String>,
    /// Number of discrete steps (0 = continuous).
    steps: u32,
    disabled: bool,
    id: Option<gpui::ElementId>,
}

impl RangeSlider {
    /// Create a new range slider with default range 0.0–1.0.
    pub fn new(theme: MaterialTheme) -> Self {
        Self {
            theme,
            start: 0.0,
            end: 1.0,
            label: None,
            steps: 0,
            disabled: false,
            id: None,
        }
    }

    /// Set the selected range (values clamped to 0.0..=1.0).
    ///
    /// If `start > end`, they will be swapped automatically.
    pub fn range(mut self, start: f32, end: f32) -> Self {
        let s = start.clamp(0.0, 1.0);
        let e = end.clamp(0.0, 1.0);
        self.start = s.min(e);
        self.end = s.max(e);
        self
    }

    /// Set an optional label.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Set discrete steps (0 = continuous).
    pub fn steps(mut self, steps: u32) -> Self {
        self.steps = steps;
        self
    }

    /// Mark as disabled.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Set an explicit element ID.
    pub fn id(mut self, id: impl Into<gpui::ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }
}

impl IntoElement for RangeSlider {
    type Element = <gpui::Stateful<gpui::Div> as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let t = self.theme;

        let active_color: Hsla = if self.disabled {
            gpui::hsla(0.0, 0.0, 0.5, 0.38)
        } else {
            color(t.primary)
        };

        let inactive_color: Hsla = if self.disabled {
            gpui::hsla(0.0, 0.0, 0.5, 0.12)
        } else {
            color(t.surface_container_highest)
        };

        let thumb_color: Hsla = if self.disabled {
            gpui::hsla(0.0, 0.0, 0.5, 0.38)
        } else {
            color(t.primary)
        };

        let label_color: Hsla = if self.disabled {
            gpui::hsla(0.0, 0.0, 0.5, 0.38)
        } else {
            color(t.on_surface)
        };

        let elem_id = self
            .id
            .unwrap_or_else(|| gpui::ElementId::Name("range-slider".into()));

        let mut container = div().id(elem_id).flex().flex_col().gap(px(4.0)).w_full();

        // Label
        if let Some(label_text) = self.label {
            container = container.child(div().text_sm().text_color(label_color).child(label_text));
        }

        // Track container
        let mut track_container = div()
            .flex_1()
            .h(px(SLIDER_TOTAL_HEIGHT))
            .flex()
            .flex_row()
            .items_center()
            .relative()
            .w_full();

        // Full inactive track
        track_container = track_container.child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .h(px(SLIDER_TRACK_HEIGHT))
                .rounded(px(SLIDER_TRACK_HEIGHT / 2.0))
                .bg(inactive_color),
        );

        // Active portion (between start and end)
        track_container = track_container.child(
            div()
                .absolute()
                .left(gpui::relative(self.start))
                .w(gpui::relative(self.end - self.start))
                .h(px(SLIDER_TRACK_HEIGHT_ACTIVE))
                .rounded(px(SLIDER_TRACK_HEIGHT_ACTIVE / 2.0))
                .bg(active_color),
        );

        // Tick marks for discrete sliders
        if self.steps > 1 {
            let mut ticks = div()
                .absolute()
                .left_0()
                .right_0()
                .h(px(SLIDER_TRACK_HEIGHT))
                .flex()
                .flex_row()
                .items_center()
                .justify_between();

            for i in 0..=self.steps {
                let pos = i as f32 / self.steps as f32;
                let in_range = pos >= self.start && pos <= self.end;
                let tick_color: Hsla = if in_range {
                    color(t.on_primary)
                } else {
                    color(t.on_surface_variant)
                };
                ticks = ticks.child(div().size(px(4.0)).rounded_full().bg(tick_color));
            }

            track_container = track_container.child(ticks);
        }

        // Start thumb
        track_container = track_container.child(
            div()
                .absolute()
                .top(px((SLIDER_TOTAL_HEIGHT - SLIDER_THUMB_SIZE) / 2.0))
                .left(gpui::relative(self.start))
                .ml(px(-(SLIDER_THUMB_SIZE / 2.0)))
                .size(px(SLIDER_THUMB_SIZE))
                .rounded_full()
                .bg(thumb_color),
        );

        // End thumb
        track_container = track_container.child(
            div()
                .absolute()
                .top(px((SLIDER_TOTAL_HEIGHT - SLIDER_THUMB_SIZE) / 2.0))
                .left(gpui::relative(self.end))
                .ml(px(-(SLIDER_THUMB_SIZE / 2.0)))
                .size(px(SLIDER_THUMB_SIZE))
                .rounded_full()
                .bg(thumb_color),
        );

        container = container.child(track_container);

        // Range labels below
        let start_pct = (self.start * 100.0).round() as i32;
        let end_pct = (self.end * 100.0).round() as i32;
        container = container.child(
            div()
                .flex()
                .flex_row()
                .justify_between()
                .text_xs()
                .text_color(rgb(t.on_surface_variant))
                .child(format!("{}%", start_pct))
                .child(format!("{}%", end_pct)),
        );

        container.into_element()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  RadioGroup (convenience)
// ═══════════════════════════════════════════════════════════════════════════════

/// A convenience wrapper that renders a vertical group of [`Radio`] buttons.
///
/// This is not a distinct MD3 component but a common pattern — a column
/// of radio buttons where only one is selected at a time.
///
/// # Example
///
/// ```rust,ignore
/// let group = RadioGroup::new(theme)
///     .option("Small", selected == 0, cx.listener(|this, _, _, cx| { this.size = 0; cx.notify(); }))
///     .option("Medium", selected == 1, cx.listener(|this, _, _, cx| { this.size = 1; cx.notify(); }))
///     .option("Large", selected == 2, cx.listener(|this, _, _, cx| { this.size = 2; cx.notify(); }));
/// ```
pub struct RadioGroup {
    theme: MaterialTheme,
    options: Vec<(String, bool, ClickHandler)>,
    disabled: bool,
}

impl RadioGroup {
    /// Create a new radio group.
    pub fn new(theme: MaterialTheme) -> Self {
        Self {
            theme,
            options: Vec::new(),
            disabled: false,
        }
    }

    /// Add a radio option with a label, selection state, and click handler.
    pub fn option(
        mut self,
        label: impl Into<String>,
        selected: bool,
        on_select: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.options
            .push((label.into(), selected, Box::new(on_select)));
        self
    }

    /// Mark all radio buttons in the group as disabled.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl IntoElement for RadioGroup {
    type Element = <gpui::Div as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let t = self.theme;
        let disabled = self.disabled;

        let mut col = div().flex().flex_col().gap(px(4.0));

        for (index, (label, selected, handler)) in self.options.into_iter().enumerate() {
            let radio = Radio::new(t)
                .selected(selected)
                .label(label)
                .disabled(disabled)
                .on_select(handler)
                .id(gpui::ElementId::Name(format!("radio-group-{index}").into()));
            col = col.child(radio);
        }

        col.into_element()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Demo / showcase
// ═══════════════════════════════════════════════════════════════════════════════

/// Renders a comprehensive showcase of all form controls.
///
/// Shows checkboxes, radio buttons, switches, sliders, and range sliders
/// in various states (normal, disabled, error, etc.).
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::material::controls;
///
/// let demo = controls::controls_demo(true); // dark mode
/// ```
pub fn controls_demo(dark: bool) -> impl IntoElement {
    let theme = MaterialTheme::from_appearance(dark);

    div()
        .flex()
        .flex_col()
        .gap_4()
        .w_full()
        // Checkboxes
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("CHECKBOXES"),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            Checkbox::new(theme)
                                .checked(true)
                                .label("Checked")
                                .id("cb-checked"),
                        )
                        .child(
                            Checkbox::new(theme)
                                .checked(false)
                                .label("Unchecked")
                                .id("cb-unchecked"),
                        )
                        .child(
                            Checkbox::new(theme)
                                .indeterminate()
                                .label("Indeterminate")
                                .id("cb-indeterminate"),
                        )
                        .child(
                            Checkbox::new(theme)
                                .checked(true)
                                .label("Disabled checked")
                                .disabled(true)
                                .id("cb-disabled-on"),
                        )
                        .child(
                            Checkbox::new(theme)
                                .checked(false)
                                .label("Disabled unchecked")
                                .disabled(true)
                                .id("cb-disabled-off"),
                        )
                        .child(
                            Checkbox::new(theme)
                                .checked(false)
                                .label("Error state")
                                .error(true)
                                .id("cb-error"),
                        ),
                ),
        )
        // Radio buttons
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("RADIO BUTTONS"),
                )
                .child(
                    RadioGroup::new(theme)
                        .option("Option A (selected)", true, |_, _, _| {})
                        .option("Option B", false, |_, _, _| {})
                        .option("Option C", false, |_, _, _| {}),
                )
                .child(
                    div().flex().flex_col().gap_1().child(
                        Radio::new(theme)
                            .selected(true)
                            .label("Disabled selected")
                            .disabled(true)
                            .id("radio-disabled"),
                    ),
                ),
        )
        // Switches
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("SWITCHES"),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(Switch::new(theme).on(true).label("On").id("sw-on"))
                        .child(Switch::new(theme).on(false).label("Off").id("sw-off"))
                        .child(
                            Switch::new(theme)
                                .on(true)
                                .with_icons()
                                .label("On with icon")
                                .id("sw-on-icon"),
                        )
                        .child(
                            Switch::new(theme)
                                .on(false)
                                .with_icons()
                                .label("Off with icon")
                                .id("sw-off-icon"),
                        )
                        .child(
                            Switch::new(theme)
                                .on(true)
                                .label("Disabled on")
                                .disabled(true)
                                .id("sw-disabled-on"),
                        )
                        .child(
                            Switch::new(theme)
                                .on(false)
                                .label("Disabled off")
                                .disabled(true)
                                .id("sw-disabled-off"),
                        ),
                ),
        )
        // Sliders
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("SLIDERS"),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(
                            Slider::new(theme)
                                .value(0.4)
                                .label("Volume")
                                .id("slider-basic"),
                        )
                        .child(
                            Slider::new(theme)
                                .value(0.7)
                                .label("Brightness")
                                .show_value_label(true)
                                .id("slider-label"),
                        )
                        .child(
                            Slider::new(theme)
                                .value(0.6)
                                .label("Quality (discrete)")
                                .steps(5)
                                .range_labels("Low", "High")
                                .id("slider-discrete"),
                        )
                        .child(
                            Slider::new(theme)
                                .value(0.5)
                                .label("Disabled")
                                .disabled(true)
                                .id("slider-disabled"),
                        ),
                ),
        )
        // Range sliders
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("RANGE SLIDERS"),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(
                            RangeSlider::new(theme)
                                .range(0.2, 0.8)
                                .label("Price range")
                                .id("range-basic"),
                        )
                        .child(
                            RangeSlider::new(theme)
                                .range(0.0, 0.6)
                                .label("Temperature (discrete)")
                                .steps(6)
                                .id("range-discrete"),
                        )
                        .child(
                            RangeSlider::new(theme)
                                .range(0.3, 0.7)
                                .label("Disabled range")
                                .disabled(true)
                                .id("range-disabled"),
                        ),
                ),
        )
}
