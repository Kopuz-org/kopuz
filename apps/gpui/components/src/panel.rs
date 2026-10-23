//! A titled block of an editor, and the dot that says its contents are not saved.

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div,
};

use crate::style;

/// The dot a changed value wears, so an edit nobody has kept is visible.
#[derive(IntoElement, Default)]
pub struct DirtyDot;

impl RenderOnce for DirtyDot {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .size_1p5()
            .rounded_full()
            .flex_shrink_0()
            .bg(cx.theme().primary)
    }
}

/// `bg-white/5 rounded-xl p-5`: one block of an editor, with a label that wears the dirty dot.
#[derive(IntoElement)]
pub struct Panel {
    label: SharedString,
    dirty: bool,
    body: Vec<AnyElement>,
}

impl Panel {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            dirty: false,
            body: Vec::new(),
        }
    }

    pub fn dirty(mut self, dirty: bool) -> Self {
        self.dirty = dirty;
        self
    }
}

impl ParentElement for Panel {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.body.extend(elements);
    }
}

impl RenderOnce for Panel {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        v_flex()
            .w_full()
            .gap_3()
            .p_5()
            .rounded(style::radius_xl())
            .bg(style::white(style::RAISED, cx))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_xs()
                            .text_color(style::slate_400(cx))
                            .child(self.label),
                    )
                    .when(self.dirty, |head| head.child(DirtyDot)),
            )
            .children(self.body)
    }
}
