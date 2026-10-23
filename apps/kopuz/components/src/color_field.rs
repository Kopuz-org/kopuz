//! A colour named by a hex string, edited with a picker or by typing, and the hex rules both share.

use gpui_kit::component::color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Colorize as _, Sizable as _, h_flex};
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, Hsla, IntoElement, ParentElement as _, Render,
    SharedString, Styled, Subscription, Window, div, rems,
};

use crate::style;

/// `text-white/70`, what a colour's name reads in.
const LABEL: f32 = 0.70;

/// Keeps only hex digits, at most six, and the leading `#` if the user typed one.
fn sanitize_hex(value: &str) -> String {
    let trimmed = value.trim();
    let digits: String = trimmed
        .chars()
        .filter(char::is_ascii_hexdigit)
        .take(6)
        .collect();
    if trimmed.starts_with('#') {
        format!("#{digits}")
    } else {
        digits
    }
}

/// `#abc` and `abcdef` both normalize to `#aabbcc` / `#abcdef`; anything else is not a colour yet.
pub fn normalize_hex(value: &str) -> Option<String> {
    let sanitized = sanitize_hex(value);
    let hex = sanitized.strip_prefix('#').unwrap_or(&sanitized);
    match hex.len() {
        3 => Some(format!(
            "#{}",
            hex.chars().flat_map(|c| [c, c]).collect::<String>()
        )),
        6 => Some(format!("#{hex}")),
        _ => None,
    }
    .map(|hex| hex.to_lowercase())
}

/// A hex string as a colour, if it is one.
pub fn parse_color(value: &str) -> Option<Hsla> {
    normalize_hex(value).and_then(|hex| Hsla::parse_hex(&hex).ok())
}

/// The field settled on a new colour, as lowercase `#rrggbb`.
#[derive(Debug, Clone)]
pub struct ColorChanged(pub String);

/// A label, a picker and a hex box kept in step; only a colour that parses is ever emitted.
pub struct ColorField {
    label: SharedString,
    value: String,
    picker: Entity<ColorPickerState>,
    hex: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ColorChanged> for ColorField {}

impl ColorField {
    pub fn new(
        label: impl Into<SharedString>,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let color = parse_color(value);
        let picker = cx.new(|cx| {
            let mut state = ColorPickerState::new(window, cx);
            if let Some(color) = color {
                state.set_value(color, window, cx);
            }
            state
        });
        let hex = cx.new(|cx| InputState::new(window, cx).default_value(value.to_string()));
        let subscriptions = vec![
            cx.subscribe_in(
                &picker,
                window,
                |this, _, event: &ColorPickerEvent, window, cx| {
                    if let ColorPickerEvent::Change(Some(color)) = event {
                        this.apply(color.to_hex().to_lowercase(), false, window, cx);
                    }
                },
            ),
            cx.subscribe_in(
                &hex,
                window,
                |this, input, event: &InputEvent, window, cx| match event {
                    InputEvent::Change => {
                        let typed = input.read(cx).value().to_string();
                        if let Some(hex) = normalize_hex(&typed) {
                            this.apply(hex, true, window, cx);
                        }
                    }
                    InputEvent::Blur => this.show_hex(window, cx),
                    _ => {}
                },
            ),
        ];
        Self {
            label: label.into(),
            value: value.to_string(),
            picker,
            hex,
            _subscriptions: subscriptions,
        }
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    /// Put a colour in from outside, rewriting both controls and saying nothing about it.
    pub fn set(&mut self, hex: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.value = hex.to_string();
        if let Some(color) = parse_color(hex) {
            self.picker
                .update(cx, |state, cx| state.set_value(color, window, cx));
        }
        self.show_hex(window, cx);
        cx.notify();
    }

    /// `from_hex` says the hex box is the one that changed, so it is left alone while typing.
    fn apply(&mut self, hex: String, from_hex: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.value == hex {
            return;
        }
        self.value = hex.clone();
        if let Some(color) = parse_color(&hex) {
            self.picker
                .update(cx, |state, cx| state.set_value(color, window, cx));
        }
        if !from_hex {
            self.show_hex(window, cx);
        }
        cx.emit(ColorChanged(hex));
        cx.notify();
    }

    fn show_hex(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.value.clone();
        self.hex
            .update(cx, |state, cx| state.set_value(value, window, cx));
    }
}

impl Render for ColorField {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .w_full()
            .gap_4()
            .items_center()
            .justify_between()
            .child(
                div()
                    .min_w_0()
                    .text_sm()
                    .text_color(style::white(LABEL, cx))
                    .child(self.label.clone()),
            )
            .child(
                h_flex()
                    .flex_shrink_0()
                    .gap_2()
                    .items_center()
                    .child(ColorPicker::new(&self.picker).small())
                    .child(div().w(rems(6.)).child(Input::new(&self.hex).small())),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_and_six_digit_hex_normalize_alike() {
        assert_eq!(normalize_hex("#ABC").as_deref(), Some("#aabbcc"));
        assert_eq!(normalize_hex("5f8aff").as_deref(), Some("#5f8aff"));
    }

    #[test]
    fn half_typed_hex_is_not_a_colour_yet() {
        assert_eq!(normalize_hex("#5f8a"), None);
        assert!(parse_color("zz").is_none());
    }
}
