//! The one keycap, and a hint pairing keycaps with what they do.

use gpui_kit::component::h_flex;
use gpui_kit::component::kbd::Kbd;
use gpui_kit::{
    App, IntoElement, Keystroke, ParentElement as _, RenderOnce, SharedString, Styled as _, Window,
    div,
};

use crate::style;

/// How a keystroke reads on a keycap, the platform's way; a chord's strokes are spaced apart.
pub fn written(keystroke: &str) -> SharedString {
    let strokes: Vec<String> = keystroke
        .split_whitespace()
        .map(|stroke| match Keystroke::parse(stroke) {
            Ok(parsed) => Kbd::format(&parsed),
            Err(_) => stroke.to_string(),
        })
        .collect();
    SharedString::from(strokes.join(" "))
}

/// One keystroke drawn as a key.
#[derive(IntoElement)]
pub struct Keycap {
    text: SharedString,
}

impl Keycap {
    /// A keystroke as the keymap writes it, such as `ctrl-shift-p`.
    pub fn new(keystroke: &str) -> Self {
        Self {
            text: written(keystroke),
        }
    }

    pub fn stroke(keystroke: &Keystroke) -> Self {
        Self {
            text: Kbd::format(keystroke).into(),
        }
    }
}

impl RenderOnce for Keycap {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .flex_shrink_0()
            .px_1p5()
            .rounded(style::radius(cx))
            .border_1()
            .border_color(style::white(style::BORDER, cx))
            .bg(style::white(style::RAISED, cx))
            .text_size(style::text_10())
            .text_color(style::white(style::TEXT_SECONDARY, cx))
            .child(self.text)
    }
}

/// Keycaps and what pressing them does, as a footer lists them.
#[derive(IntoElement)]
pub struct KeyHint {
    keys: Vec<Keycap>,
    label: SharedString,
}

impl KeyHint {
    pub fn new<'a>(
        keys: impl IntoIterator<Item = &'a str>,
        label: impl Into<SharedString>,
    ) -> Self {
        Self {
            keys: keys.into_iter().map(Keycap::new).collect(),
            label: label.into(),
        }
    }
}

impl RenderOnce for KeyHint {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        h_flex().gap_1p5().items_center().children(self.keys).child(
            div()
                .text_size(style::text_10())
                .text_color(style::white(style::TEXT_MUTED, cx))
                .child(self.label),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::written;

    #[test]
    fn a_chord_is_written_stroke_by_stroke() {
        let one = written("ctrl-k");
        let chord = written("ctrl-k ctrl-s");
        assert!(one.ends_with('K'));
        assert_eq!(chord, format!("{one} {}", written("ctrl-s")));
    }
}
