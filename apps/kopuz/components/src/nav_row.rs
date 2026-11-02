//! A row in a navigation list: a glyph, a name and a count, washed by the pointer and lit while it is the page.

use std::rc::Rc;

use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, ClickEvent, Div, ElementId, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, RenderOnce, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, Window, div,
};
use store::Target;

use crate::reorder::ReorderDropZones;
use crate::{focus, icons, menus, motion, skin, style};

const WASH: u64 = 2;

const GLYPH: gpui_kit::Pixels = gpui_kit::px(14.);

type Wire = Box<dyn FnOnce(Stateful<Div>) -> Stateful<Div>>;
type Click = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// A navigation row; `scope` names the list it is in and `id` the thing it opens.
#[derive(IntoElement)]
pub struct NavListRow {
    scope: &'static str,
    id: SharedString,
    label: SharedString,
    glyph: &'static str,
    detail: Option<SharedString>,
    active: bool,
    focused: bool,
    target: Option<Target>,
    click: Option<Click>,
    wires: Vec<Wire>,
    zones: Option<ReorderDropZones>,
}

impl NavListRow {
    pub fn new(
        scope: &'static str,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
    ) -> Self {
        Self {
            scope,
            id: id.into(),
            label: label.into(),
            glyph: icons::LIST,
            detail: None,
            active: false,
            focused: false,
            target: None,
            click: None,
            wires: Vec::new(),
            zones: None,
        }
    }

    pub fn glyph(mut self, glyph: &'static str) -> Self {
        self.glyph = glyph;
        self
    }

    /// A small count or note at the row's end.
    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// The row is the page being shown.
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    /// The keyboard cursor is on the row.
    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    pub fn menu(mut self, target: Target) -> Self {
        self.target = Some(target);
        self
    }

    pub fn on_click(
        mut self,
        click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.click = Some(Rc::new(click));
        self
    }

    /// Carry `payload` when the row is dragged, for drop strips to take.
    pub fn drag<D: Clone + Render + 'static>(mut self, payload: D) -> Self {
        self.wires.push(Box::new(move |row| {
            row.on_drag(payload, |drag, _, _, cx| cx.new(|_| drag.clone()))
        }));
        self
    }

    /// Take a drop of `D` on the row itself, lit with the drop-target colour while one is over it.
    pub fn accept<D: 'static>(
        mut self,
        handler: impl Fn(&D, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.wires.push(Box::new(move |row| {
            row.on_drop::<D>(move |drag, window, cx| handler(drag, window, cx))
                .drag_over::<D>(|style, _, _, cx| style.bg(cx.theme().drop_target))
        }));
        self
    }

    pub fn drop_zones(mut self, zones: ReorderDropZones) -> Self {
        self.zones = Some(zones);
        self
    }
}

impl RenderOnce for NavListRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (hovered, pointer) = motion::hovered(
            ElementId::from((ElementId::from(self.scope), self.id.clone())),
            window,
            cx,
        );
        let wash = motion::color(
            motion::channel(&self.id, WASH),
            match (self.active, hovered) {
                (true, _) => style::white(style::HOVER, cx),
                (false, true) => style::white(style::RAISED, cx),
                (false, false) => style::white(0., cx),
            },
            window,
            cx,
        );
        let ink = match self.active || hovered {
            true => cx.theme().foreground,
            false => style::slate_400(cx),
        };
        let click = self.click;

        let row = h_flex()
            .id(ElementId::from((
                ElementId::from(self.scope),
                SharedString::from(format!("{}-row", self.id)),
            )))
            .relative()
            .w_full()
            .h(style::list_row())
            .px_2()
            .gap_2()
            // Material's navigation items are full pills.
            .map(|row| match skin::material() {
                true => row.rounded_full().px_4(),
                false => row.rounded(style::radius_lg()),
            })
            .cursor_pointer()
            .bg(wash)
            .text_color(ink)
            .on_hover(motion::track(pointer))
            .on_click(move |event, window, cx| {
                if let Some(click) = &click {
                    click(event, window, cx);
                }
            });
        let row = self.wires.into_iter().fold(row, |row, wire| wire(row));

        let body = row
            .child(Icon::default().path(self.glyph).size(GLYPH).flex_shrink_0())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .child(self.label),
            )
            .children(self.detail.map(|detail| {
                div()
                    .flex_shrink_0()
                    .text_size(style::text_10())
                    .text_color(style::white(style::TEXT_SUBTLE, cx))
                    .child(detail)
            }))
            .when(self.focused, |row| {
                row.child(focus::ring(style::radius_lg(), cx))
            })
            .children(self.zones);

        match self.target {
            Some(target) => body.context_menu(menus::builder(target)).into_any_element(),
            None => body.into_any_element(),
        }
    }
}
