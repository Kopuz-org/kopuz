//! What follows the pointer while something is dragged: a chip naming what is being carried.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{
    App, IntoElement, ParentElement as _, RenderOnce, SharedString, Styled as _, Window, div,
};

/// The chip every drag payload renders, whatever it carries.
#[derive(IntoElement)]
pub struct DragChip {
    label: SharedString,
}

impl DragChip {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
        }
    }
}

impl RenderOnce for DragChip {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded(cx.theme().radius)
            .bg(cx.theme().popover)
            .text_color(cx.theme().popover_foreground)
            .text_sm()
            .child(self.label)
    }
}
