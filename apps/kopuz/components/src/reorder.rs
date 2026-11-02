//! The two strips over a list row that take a drop: above the row's split it lands before, below it after.

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{
    App, Div, ElementId, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    RenderOnce, Stateful, Styled as _, Window, div,
};

type Wire = Rc<dyn Fn(Stateful<Div>, usize) -> Stateful<Div>>;

/// Half a row that accepts each drag type given to it, handing the drop the insertion point it landed on.
#[derive(IntoElement)]
pub struct ReorderDropZones {
    id: ElementId,
    height: Pixels,
    split: Pixels,
    index: usize,
    wires: Vec<Wire>,
}

impl ReorderDropZones {
    /// Over the row at `index`, `height` tall: a drop above its split lands at `index`, below at `index + 1`.
    pub fn new(id: impl Into<ElementId>, height: Pixels, index: usize) -> Self {
        Self {
            id: id.into(),
            height,
            split: height / 2.,
            index,
            wires: Vec::new(),
        }
    }

    /// How far down the row a drop stops counting as "before" it; half of it unless set.
    pub fn split(mut self, split: Pixels) -> Self {
        self.split = split;
        self
    }

    /// Take drops of `D`, lit with the drop-target colour while one is over a strip.
    pub fn on_drop<D: 'static>(
        mut self,
        handler: impl Fn(&D, usize, &mut Window, &mut App) + 'static,
    ) -> Self {
        let handler = Rc::new(handler);
        self.wires.push(Rc::new(move |zone, at| {
            let handler = handler.clone();
            zone.on_drop::<D>(move |drag, window, cx| handler(drag, at, window, cx))
                .drag_over::<D>(|style, _, _, cx| style.bg(cx.theme().drop_target))
        }));
        self
    }
}

impl RenderOnce for ReorderDropZones {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let key = Arc::new(self.id);
        let strip = |name: &'static str, at: usize| {
            let zone = div()
                .id(ElementId::NamedChild(key.clone(), name.into()))
                .absolute()
                .left_0()
                .right_0();
            self.wires.iter().fold(zone, |zone, wire| wire(zone, at))
        };

        div()
            .absolute()
            .inset_0()
            .child(strip("before", self.index).top_0().h(self.split))
            .child(
                strip("after", self.index + 1)
                    .bottom_0()
                    .h(self.height - self.split),
            )
    }
}
