//! Material Design 3 interactive text input component.
//!
//! Provides an outlined text field that shows the software keyboard on tap,
//! receives text input, and displays the current value with a cursor that
//! can be positioned within the text.

use gpui::{div, prelude::*, px, rgb, ElementId, MouseButton, MouseDownEvent};

use super::theme::MaterialTheme;

/// Blend two RGB colors. `t` is 0.0–1.0 where 0.0 = `a`, 1.0 = `b`.
fn blend_rgb(a: u32, b: u32, t: f32) -> u32 {
    let ar = ((a >> 16) & 0xFF) as f32;
    let ag = ((a >> 8) & 0xFF) as f32;
    let ab = (a & 0xFF) as f32;
    let br = ((b >> 16) & 0xFF) as f32;
    let bg = ((b >> 8) & 0xFF) as f32;
    let bb = (b & 0xFF) as f32;
    let r = (ar + (br - ar) * t) as u32;
    let g = (ag + (bg - ag) * t) as u32;
    let b_val = (ab + (bb - ab) * t) as u32;
    (r << 16) | (g << 8) | b_val
}

/// An interactive Material Design 3 text input field.
///
/// When tapped, this component triggers the software keyboard via
/// `gpui_mobile::show_keyboard()`. Text state is managed externally
/// by the parent component through the `on_change` callback.
///
/// # Example
///
/// ```rust,ignore
/// TextInput::new("email", theme)
///     .label("Email")
///     .value(&self.email)
///     .placeholder("user@example.com")
///     .cursor(field.cursor)
///     .selection(field.normalized_selection())
///     .on_tap_notify(|event| { /* handle tap with position */ })
/// ```
#[allow(clippy::type_complexity)]
pub struct TextInput<V: 'static> {
    id: ElementId,
    theme: MaterialTheme,
    label: Option<&'static str>,
    value: String,
    placeholder: &'static str,
    error: bool,
    error_text: Option<&'static str>,
    focused: bool,
    keyboard_type: crate::KeyboardType,
    cursor_position: usize,
    selection: Option<(usize, usize)>,
    on_tap: Option<Box<dyn Fn(&mut V, &MouseDownEvent, &mut gpui::Window, &mut gpui::Context<V>)>>,
    /// Simple tap callback that receives the MouseDownEvent for tap position.
    on_tap_simple: Option<std::rc::Rc<dyn Fn(&MouseDownEvent)>>,
}

impl<V: 'static> TextInput<V> {
    /// Create a new text input with the given ID and theme.
    pub fn new(id: impl Into<ElementId>, theme: MaterialTheme) -> Self {
        Self {
            id: id.into(),
            theme,
            label: None,
            value: String::new(),
            placeholder: "",
            error: false,
            error_text: None,
            focused: false,
            keyboard_type: crate::KeyboardType::Default,
            cursor_position: 0,
            selection: None,
            on_tap: None,
            on_tap_simple: None,
        }
    }

    /// Set the floating label text.
    pub fn label(mut self, label: &'static str) -> Self {
        self.label = Some(label);
        self
    }

    /// Set the current text value.
    pub fn value(mut self, value: &str) -> Self {
        self.value = value.to_string();
        self
    }

    /// Set the placeholder text shown when empty.
    pub fn placeholder(mut self, placeholder: &'static str) -> Self {
        self.placeholder = placeholder;
        self
    }

    /// Mark the field as having an error.
    pub fn error(mut self, error: bool) -> Self {
        self.error = error;
        self
    }

    /// Set the error helper text shown below the field.
    pub fn error_text(mut self, text: &'static str) -> Self {
        self.error_text = Some(text);
        self
    }

    /// Mark the field as focused (shows active border color).
    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    /// Set the keyboard type to present when this field is focused.
    pub fn keyboard_type(mut self, kt: crate::KeyboardType) -> Self {
        self.keyboard_type = kt;
        self
    }

    /// Set the cursor byte offset within the text.
    pub fn cursor(mut self, position: usize) -> Self {
        self.cursor_position = position;
        self
    }

    /// Set the normalized selection range `(min, max)` in byte offsets.
    pub fn selection(mut self, sel: Option<(usize, usize)>) -> Self {
        self.selection = sel;
        self
    }

    /// Set a callback for when the field is tapped.
    ///
    /// The callback should call `gpui_mobile::show_keyboard()` and set
    /// the focused field state.
    pub fn on_tap(
        mut self,
        handler: impl Fn(&mut V, &MouseDownEvent, &mut gpui::Window, &mut gpui::Context<V>) + 'static,
    ) -> Self {
        self.on_tap = Some(Box::new(handler));
        self
    }

    /// Set a simple tap callback that receives the `MouseDownEvent` for tap
    /// position. Does NOT lease the parent entity — use this instead of
    /// `on_tap` to avoid entity lease conflicts.
    pub fn on_tap_notify(mut self, handler: impl Fn(&MouseDownEvent) + 'static) -> Self {
        self.on_tap_simple = Some(std::rc::Rc::new(handler));
        self
    }

    /// Build the element. Must be called with a context to wire up event handlers.
    pub fn render(mut self, cx: &mut gpui::Context<V>) -> impl IntoElement {
        let t = self.theme;
        let border_color = if self.error {
            t.error
        } else if self.focused {
            t.primary
        } else {
            t.outline
        };
        let label_color = if self.error {
            t.error
        } else if self.focused {
            t.primary
        } else {
            t.on_surface_variant
        };

        let has_value = !self.value.is_empty();
        let text_color = if has_value {
            t.on_surface
        } else {
            t.on_surface_variant
        };

        let border_width = if self.focused { 2.0 } else { 1.0 };

        let mut field = div().id(self.id).flex().flex_col().gap_1().w_full();

        // Label
        if let Some(label) = self.label {
            field = field.child(
                div()
                    .text_xs()
                    .text_color(rgb(label_color))
                    .child(label.to_string()),
            );
        }

        // Input container
        let mut input_box = div()
            .px_3()
            .py_2()
            .rounded_md()
            .border_color(rgb(border_color))
            .bg(rgb(t.surface));

        if border_width > 1.5 {
            input_box = input_box.border_2();
        } else {
            input_box = input_box.border_1();
        }

        // Text content with cursor/selection
        let cursor_pos = self.cursor_position;
        let selection = self.selection;
        let value = std::mem::take(&mut self.value);
        let focused = self.focused;
        let placeholder = self.placeholder;

        let text_row = if has_value && focused {
            Self::render_text_with_cursor_static(&value, cursor_pos, selection, text_color, t)
        } else {
            // No value or not focused — show placeholder or plain text
            let display_text = if has_value {
                value.clone()
            } else {
                placeholder.to_string()
            };
            let mut row = div()
                .flex()
                .flex_row()
                .items_center()
                .text_sm()
                .text_color(rgb(text_color))
                .child(display_text);

            // Show cursor at end when focused on empty field
            if focused {
                row = row.child(div().w(px(2.0)).h(px(16.0)).bg(rgb(t.primary)).ml_px());
            }
            row
        };

        input_box = input_box.child(text_row);

        // Wire up tap handler — prefer on_tap_simple (no entity lease) over on_tap
        if let Some(handler) = self.on_tap_simple {
            let handler_clone = handler.clone();
            input_box = input_box.on_mouse_down(
                MouseButton::Left,
                move |event: &MouseDownEvent, _window: &mut gpui::Window, _cx: &mut gpui::App| {
                    log::info!("TextInput: on_tap_simple handler firing");
                    (handler_clone)(event);
                },
            );
        } else if let Some(on_tap) = self.on_tap {
            let on_tap = std::rc::Rc::new(on_tap);
            let on_tap_clone = on_tap.clone();
            input_box = input_box.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    (on_tap_clone)(this, event, window, cx);
                }),
            );
        }

        field = field.child(input_box);

        // Error text
        if let Some(error_text) = self.error_text {
            if self.error {
                field = field.child(
                    div()
                        .text_xs()
                        .text_color(rgb(t.error))
                        .child(error_text.to_string()),
                );
            }
        }

        field
    }

    /// Render the text split around the cursor position, with optional
    /// selection highlighting.
    fn render_text_with_cursor_static(
        value: &str,
        cursor_position: usize,
        selection: Option<(usize, usize)>,
        text_color: u32,
        t: MaterialTheme,
    ) -> gpui::Div {
        let cursor_pos = cursor_position.min(value.len());

        if let Some((sel_min, sel_max)) = selection {
            // Clamp selection to text length
            let sel_min = sel_min.min(value.len());
            let sel_max = sel_max.min(value.len());

            let before_sel = &value[..sel_min];
            let selected = &value[sel_min..sel_max];
            let after_sel = &value[sel_max..];

            // Selection highlight color: blend primary at 30% with surface
            let highlight_bg = blend_rgb(t.surface, t.primary, 0.3);

            let mut row = div()
                .flex()
                .flex_row()
                .items_center()
                .text_sm()
                .text_color(rgb(text_color));

            if !before_sel.is_empty() {
                row = row.child(div().child(before_sel.to_string()));
            }

            if !selected.is_empty() {
                row = row.child(
                    div()
                        .bg(rgb(highlight_bg))
                        .rounded_sm()
                        .px(px(1.0))
                        .child(selected.to_string()),
                );
            }

            // Cursor bar at selection edge
            row = row.child(div().w(px(2.0)).h(px(16.0)).bg(rgb(t.primary)));

            if !after_sel.is_empty() {
                row = row.child(div().child(after_sel.to_string()));
            }

            row
        } else {
            // No selection — split text at cursor
            let before = &value[..cursor_pos];
            let after = &value[cursor_pos..];

            let mut row = div()
                .flex()
                .flex_row()
                .items_center()
                .text_sm()
                .text_color(rgb(text_color));

            if !before.is_empty() {
                row = row.child(div().child(before.to_string()));
            }

            // Cursor bar
            row = row.child(div().w(px(2.0)).h(px(16.0)).bg(rgb(t.primary)));

            if !after.is_empty() {
                row = row.child(div().child(after.to_string()));
            }

            row
        }
    }
}
