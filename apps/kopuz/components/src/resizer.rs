//! The one drag handle on an edge: pull it and what it sits on takes the width it is left at.

use std::rc::Rc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, DragMoveEvent, InteractiveElement as _, IntoElement,
    MouseButton, MouseDownEvent, Pixels, Render, RenderOnce, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

use crate::{motion, style};

const FRAME: u64 = 0;
const HOVER: u64 = 1;
const LINE: u64 = 2;
const START: u64 = 3;

/// `w-1.5`: wide enough to grab without covering what it borders.
const THICKNESS: Pixels = px(6.);

/// Which edge of its parent the handle sits on; dragging away from the parent grows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Left,
    Right,
}

/// The payload of an edge drag, naming the handle it started on, since every handle hears every move.
#[derive(Clone)]
struct EdgeDrag(SharedString);

impl Render for EdgeDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui_kit::Empty
    }
}

type OnResize = Rc<dyn Fn(Pixels, &mut Window, &mut App)>;

/// A thin strip on one edge of an absolutely positioned parent, reporting the size a drag leaves it at.
#[derive(IntoElement)]
pub struct EdgeResizer {
    id: SharedString,
    edge: Edge,
    size: Pixels,
    range: Option<(Pixels, Pixels)>,
    straddle: bool,
    resize: Option<OnResize>,
}

impl EdgeResizer {
    /// `size` is what the parent measures now, along the axis the handle drags.
    pub fn new(id: impl Into<SharedString>, edge: Edge, size: Pixels) -> Self {
        Self {
            id: id.into(),
            edge,
            size,
            range: None,
            straddle: false,
            resize: None,
        }
    }

    /// The narrowest and widest the parent may be dragged to.
    pub fn range(mut self, min: Pixels, max: Pixels) -> Self {
        self.range = Some((min, max));
        self
    }

    /// Centre the strip on the edge rather than inside it, as a column's divider sits.
    pub fn straddle(mut self, straddle: bool) -> Self {
        self.straddle = straddle;
        self
    }

    pub fn on_resize(mut self, resize: impl Fn(Pixels, &mut Window, &mut App) + 'static) -> Self {
        self.resize = Some(Rc::new(resize));
        self
    }
}

impl RenderOnce for EdgeResizer {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self.id;
        let (hovered, pointer) = motion::hovered(motion::channel(&id, HOVER), window, cx);
        let line = motion::color(
            motion::channel(&id, LINE),
            style::white(if hovered { style::HOVER } else { 0. }, cx),
            window,
            cx,
        );
        let start = window.use_keyed_state(motion::channel(&id, START), cx, |_, _| {
            None::<(Pixels, Pixels)>
        });
        let inset = match self.straddle {
            true => -THICKNESS / 2.,
            false => px(0.),
        };
        let (edge, size, range, resize) = (self.edge, self.size, self.range, self.resize);
        let pressed = start.clone();
        let mine = id.clone();
        div()
            .id(motion::channel(&id, FRAME))
            .absolute()
            .top_0()
            .bottom_0()
            .map(|strip| match edge {
                Edge::Left => strip.left(inset),
                Edge::Right => strip.right(inset),
            })
            .w(THICKNESS)
            .occlude()
            .cursor_col_resize()
            .bg(line)
            .on_hover(motion::track(pointer))
            .on_drag(EdgeDrag(id.clone()), |drag, _, _, cx| {
                cx.stop_propagation();
                cx.new(|_| drag.clone())
            })
            .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, _, cx| {
                let from = event.position.x;
                pressed.update(cx, |held, _| *held = Some((from, size)));
                cx.stop_propagation();
            })
            .when_some(resize, |strip, resize| {
                strip.on_drag_move(move |event: &DragMoveEvent<EdgeDrag>, window, cx| {
                    if event.drag(cx).0 != mine {
                        return;
                    }
                    let Some((from, base)) = *start.read(cx) else {
                        return;
                    };
                    let moved = event.event.position.x - from;
                    let size = match edge {
                        Edge::Right => base + moved,
                        Edge::Left => base - moved,
                    };
                    let size = match range {
                        Some((min, max)) => size.clamp(min, max),
                        None => size,
                    };
                    resize(size, window, cx)
                })
            })
    }
}
