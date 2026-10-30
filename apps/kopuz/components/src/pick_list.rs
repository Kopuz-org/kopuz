//! A scrolling list of things to pick one of, and the name box that makes a new one on the spot.

use std::rc::Rc;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Pixels, RenderOnce,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

use crate::format::t;
use crate::{icons, motion, style};

const HOVER: u64 = 30;
const FILL: u64 = 31;
const ROW: u64 = 32;
const LIST: u64 = 33;

type OnPick = Rc<dyn Fn(&SharedString, &mut Window, &mut App)>;
type OnCreate = Rc<dyn Fn(&mut Window, &mut App)>;

/// One row to pick; `id` is what the owner gets back.
pub struct PickItem {
    id: SharedString,
    label: SharedString,
    icon: Option<&'static str>,
}

impl PickItem {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
        }
    }

    /// A glyph before the label, drawn in the warning tone.
    pub fn icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }
}

/// Rows to pick from, scrolling past a height.
#[derive(IntoElement)]
pub struct PickList {
    id: SharedString,
    items: Vec<PickItem>,
    raised: bool,
    max_height: Pixels,
    on_pick: Option<OnPick>,
}

impl PickList {
    pub fn new(id: impl Into<SharedString>, items: impl IntoIterator<Item = PickItem>) -> Self {
        Self {
            id: id.into(),
            items: items.into_iter().collect(),
            raised: false,
            max_height: px(240.),
            on_pick: None,
        }
    }

    /// Rows rest on a raised fill with roomier padding, as a list of named things reads best.
    pub fn raised(mut self, raised: bool) -> Self {
        self.raised = raised;
        self
    }

    pub fn max_height(mut self, max_height: Pixels) -> Self {
        self.max_height = max_height;
        self
    }

    pub fn on_pick(
        mut self,
        pick: impl Fn(&SharedString, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_pick = Some(Rc::new(pick));
        self
    }
}

impl RenderOnce for PickList {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let raised = self.raised;
        let rows = self.items.into_iter().map(|item| {
            let (hot, hover) = motion::hovered(motion::channel(&item.id, HOVER), window, cx);
            let rest = match raised {
                true => style::RAISED,
                false => 0.,
            };
            let fill = motion::color(
                motion::channel(&item.id, FILL),
                style::white(if hot { style::HOVER } else { rest }, cx),
                window,
                cx,
            );
            let pick = self.on_pick.clone();
            let picked = item.id.clone();
            h_flex()
                .id(motion::channel(&item.id, ROW))
                .gap_2()
                .text_sm()
                .rounded(style::radius_lg())
                .bg(fill)
                .when(raised, |row| row.p_3())
                .when(!raised, |row| row.px_3().py_2())
                .on_hover(motion::track(hover))
                .children(item.icon.map(|icon| {
                    icons::icon(icon)
                        .size(px(12.))
                        .text_color(cx.theme().warning)
                }))
                .child(item.label)
                .on_click(move |_, window, cx| {
                    if let Some(pick) = pick.as_ref() {
                        pick(&picked, window, cx);
                    }
                })
        });
        v_flex()
            .id(motion::channel(&self.id, LIST))
            .gap(match raised {
                true => px(8.),
                false => px(4.),
            })
            .max_h(self.max_height)
            .overflow_y_scroll()
            .children(rows)
    }
}

/// A name box with a Create button beside it.
#[derive(IntoElement)]
pub struct QuickCreate {
    input: Entity<InputState>,
    icon: Option<&'static str>,
    on_create: Option<OnCreate>,
}

impl QuickCreate {
    pub fn new(input: &Entity<InputState>) -> Self {
        Self {
            input: input.clone(),
            icon: None,
            on_create: None,
        }
    }

    pub fn icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn on_create(mut self, create: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_create = Some(Rc::new(create));
        self
    }
}

impl RenderOnce for QuickCreate {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let create = self.on_create;
        h_flex()
            .gap_2()
            .child(div().flex_1().child(Input::new(&self.input)))
            .child(
                Button::new(("quick-create", self.input.entity_id().as_u64()))
                    .primary()
                    .text_sm()
                    .when_some(self.icon, |button, icon| button.icon(icons::icon(icon)))
                    .label(t("create"))
                    .on_click(move |_, window, cx| {
                        if let Some(create) = create.as_ref() {
                            create(window, cx);
                        }
                    }),
            )
    }
}
