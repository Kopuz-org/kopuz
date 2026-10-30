//! A column docked beside the content: a scrim, a ruled header with a close button, a body and a drag edge.

use std::rc::Rc;

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, IntoElement, ParentElement as _, Pixels, RenderOnce, SharedString,
    Styled as _, Window, div, px,
};

use crate::icon_button::IconButton;
use crate::resizer::{Edge, EdgeResizer};
use crate::{format::t, icons, style};

// `bg-black/40`: the column darkens whatever the window paints behind it.
const SCRIM: f32 = 0.4;

type Press = Rc<dyn Fn(&mut Window, &mut App)>;
type Resize = Rc<dyn Fn(Pixels, &mut Window, &mut App)>;

/// How roomy the header is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DockSize {
    /// A tab strip's header: the wider padding, a full-size close button.
    #[default]
    Roomy,
    /// A title's header: tighter, with a small close button.
    Compact,
}

/// A docked column; `edge` is the side it borders the content on, where the rule and the drag handle sit.
#[derive(IntoElement)]
pub struct DockPanel {
    id: SharedString,
    edge: Edge,
    width: Option<Pixels>,
    size: DockSize,
    header: Option<AnyElement>,
    body: Option<AnyElement>,
    close: Option<Press>,
    resize: Option<(Pixels, Pixels, Pixels, Resize)>,
}

impl DockPanel {
    pub fn new(id: impl Into<SharedString>, edge: Edge) -> Self {
        Self {
            id: id.into(),
            edge,
            width: None,
            size: DockSize::Roomy,
            header: None,
            body: None,
            close: None,
            resize: None,
        }
    }

    /// A fixed width; without one the column fills the slot it is put in.
    pub fn width(mut self, width: Pixels) -> Self {
        self.width = Some(width);
        self
    }

    pub fn size(mut self, size: DockSize) -> Self {
        self.size = size;
        self
    }

    /// What the header holds beside the close button.
    pub fn header(mut self, header: impl IntoElement) -> Self {
        self.header = Some(header.into_any_element());
        self
    }

    pub fn body(mut self, body: impl IntoElement) -> Self {
        self.body = Some(body.into_any_element());
        self
    }

    pub fn on_close(mut self, close: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.close = Some(Rc::new(close));
        self
    }

    /// Let a drag on the bordering edge size the column, now `current` wide, between `min` and `max`.
    pub fn resizable(
        mut self,
        current: Pixels,
        min: Pixels,
        max: Pixels,
        resize: impl Fn(Pixels, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.resize = Some((current, min, max, Rc::new(resize)));
        self
    }
}

impl RenderOnce for DockPanel {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self.id;
        let (pad_x, pad_y, rule, button, glyph) = match self.size {
            DockSize::Roomy => (
                px(16.),
                px(12.),
                style::BORDER,
                style::chrome().round_button,
                px(14.),
            ),
            DockSize::Compact => (px(12.), px(8.), style::BORDER_SOFT, px(24.), px(12.)),
        };
        let close = self.close;
        let edge = self.edge;
        let width = self.width;

        v_flex()
            .relative()
            .h_full()
            .map(|panel| match width {
                Some(width) => panel.w(width).flex_shrink_0(),
                None => panel.w_full(),
            })
            .bg(style::black(SCRIM, cx))
            .map(|panel| match edge {
                Edge::Left => panel.border_l_1(),
                Edge::Right => panel.border_r_1(),
            })
            .border_color(style::white(style::BORDER_SOFT, cx))
            .child(
                h_flex()
                    .w_full()
                    .flex_shrink_0()
                    .justify_between()
                    .items_center()
                    .px(pad_x)
                    .py(pad_y)
                    .border_b_1()
                    .border_color(style::white(rule, cx))
                    .children(self.header)
                    .child(
                        IconButton::new(SharedString::from(format!("{id}-close")), icons::CLOSE)
                            .size(button)
                            .glyph(glyph)
                            .tooltip(t("close"))
                            .on_click(move |_, window, cx| {
                                if let Some(close) = &close {
                                    close(window, cx);
                                }
                            }),
                    ),
            )
            .child(div().flex_1().min_h_0().w_full().children(self.body))
            .when_some(self.resize, |panel, (current, min, max, resize)| {
                panel.child(
                    EdgeResizer::new(SharedString::from(format!("{id}-resize")), edge, current)
                        .range(min, max)
                        .on_resize(move |width, window, cx| resize(width, window, cx)),
                )
            })
    }
}
