//! A checkable row with a trailing action, and the summary line that opens and closes a section.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::{Disableable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, IntoElement, ParentElement, RenderOnce, SharedString, Styled,
    Window, div,
};

use crate::{icons, style};

type OnToggle = Rc<dyn Fn(&bool, &mut Window, &mut App)>;

/// A checkbox and a name on the raised row of a list; an unchecked name is struck through.
#[derive(IntoElement)]
pub struct CheckRow {
    id: SharedString,
    label: SharedString,
    checked: bool,
    locked: bool,
    trailing: Option<AnyElement>,
    on_toggle: Option<OnToggle>,
}

impl CheckRow {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            checked: false,
            locked: false,
            trailing: None,
            on_toggle: None,
        }
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn locked(mut self, locked: bool) -> Self {
        self.locked = locked;
        self
    }

    /// What sits at the row's end, such as a delete button.
    pub fn trailing(mut self, trailing: impl IntoElement) -> Self {
        self.trailing = Some(trailing.into_any_element());
        self
    }

    pub fn on_toggle(mut self, toggle: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Rc::new(toggle));
        self
    }
}

impl RenderOnce for CheckRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let toggle = self.on_toggle;
        h_flex()
            .w_full()
            .gap_3()
            .items_center()
            .p_2()
            .rounded(style::radius(cx))
            .bg(style::white(style::RAISED, cx))
            .child(
                Checkbox::new(self.id)
                    .checked(self.checked)
                    .disabled(self.locked)
                    .on_click(move |checked, window, cx| {
                        if let Some(toggle) = &toggle {
                            toggle(checked, window, cx);
                        }
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .map(|label| match self.checked {
                        true => label.text_color(style::slate_400(cx)),
                        false => label.line_through().text_color(style::slate_500(cx)),
                    })
                    .child(self.label),
            )
            .children(self.trailing)
    }
}

type OnClick = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// A summary line with a chevron; its body shows only while it is open.
#[derive(IntoElement)]
pub struct Disclosure {
    id: SharedString,
    summary: SharedString,
    open: bool,
    on_toggle: Option<OnClick>,
    body: Vec<AnyElement>,
}

impl Disclosure {
    pub fn new(id: impl Into<SharedString>, summary: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            summary: summary.into(),
            open: false,
            on_toggle: None,
            body: Vec::new(),
        }
    }

    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    pub fn on_toggle(
        mut self,
        toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_toggle = Some(Rc::new(toggle));
        self
    }
}

impl ParentElement for Disclosure {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.body.extend(elements);
    }
}

impl RenderOnce for Disclosure {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let toggle = self.on_toggle;
        v_flex()
            .w_full()
            .gap_3()
            .child(
                Button::new(self.id)
                    .ghost()
                    .justify_between()
                    .w_full()
                    .label(self.summary)
                    .icon(icons::icon(match self.open {
                        true => icons::CHEVRON_UP,
                        false => icons::CHEVRON_DOWN,
                    }))
                    .on_click(move |event, window, cx| {
                        if let Some(toggle) = &toggle {
                            toggle(event, window, cx);
                        }
                    }),
            )
            .when(self.open, |section| section.children(self.body))
    }
}
