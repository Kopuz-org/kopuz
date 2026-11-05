//! The "E" a source puts beside an explicit song or release.

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    App, ElementId, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    RenderOnce, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

use crate::format::t;
use crate::style;

/// `opacity: 0.55` on the Dioxus badge: legible, quieter than the title beside it.
const INK: f32 = 0.55;

#[derive(IntoElement)]
pub struct ExplicitBadge {
    id: ElementId,
}

impl ExplicitBadge {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self { id: id.into() }
    }
}

impl RenderOnce for ExplicitBadge {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let label = t("explicit");
        div()
            .id(self.id)
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            // The badge is a measured 14px square with a 9px glyph, as drawn upstream.
            .size(px(14.))
            .rounded(px(3.))
            .bg(style::white(INK, cx))
            .text_color(style::black(1., cx))
            .text_size(px(9.))
            .font_weight(FontWeight::BOLD)
            .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
            // TODO(i18n): the Dioxus badge draws a literal "E" too.
            .child("E")
    }
}
