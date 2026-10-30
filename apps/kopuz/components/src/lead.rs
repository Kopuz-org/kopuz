//! A row's lead: its number, which steps aside for a play glyph while the row is under the pointer.

use gpui_kit::{
    AnyElement, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    SharedString, Styled as _, div, px,
};

use crate::icons;

/// The lead's glyph, playing or play.
pub const GLYPH: Pixels = px(12.);

/// What a lead cell holds: the playing glyph, else its number; the cell has to be `relative`.
pub fn lead(
    group: &'static str,
    playing: bool,
    number: Option<SharedString>,
    ink: Hsla,
) -> Vec<AnyElement> {
    match (playing, number) {
        (true, _) => vec![
            icons::icon(icons::VOLUME)
                .size(GLYPH)
                .text_color(ink)
                .into_any_element(),
        ],
        (false, Some(number)) => number_or_play(group, number, ink).into(),
        (false, None) => Vec::new(),
    }
}

/// The number and the glyph that replaces it on hover.
fn number_or_play(group: &'static str, number: SharedString, color: Hsla) -> [AnyElement; 2] {
    [
        div()
            .text_xs()
            .text_color(color)
            .group_hover(group, |number| number.invisible())
            .child(number)
            .into_any_element(),
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .invisible()
            .group_hover(group, |play| play.visible())
            .child(icons::icon(icons::PLAY).size(GLYPH).text_color(color))
            .into_any_element(),
    ]
}
