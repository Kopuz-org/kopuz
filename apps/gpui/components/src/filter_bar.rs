//! The search box and the filter chips beside it: toggles, menus that pick a value, and the way to clear them all.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::InputState;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::component::{Sizable as _, h_flex};
use gpui_kit::{
    AnyElement, App, ClickEvent, Entity, IntoElement, ParentElement as _, RenderOnce, SharedString,
    Styled as _, Window, div, rems,
};

use crate::format::t;
use crate::{ChipSize, ChipTone, SearchField, icons};

type OnToggle = Rc<dyn Fn(&mut Window, &mut App)>;
type OnPick = Rc<dyn Fn(Option<String>, &mut Window, &mut App)>;
type OnClear = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// One answer a menu chip offers: the value it sets (none clears), what to call it, and whether it is on.
#[derive(Clone)]
pub struct FilterEntry {
    pub value: Option<String>,
    pub label: SharedString,
    pub checked: bool,
}

enum Kind {
    Toggle(OnToggle),
    Menu(Vec<FilterEntry>, OnPick),
}

/// One filter: lit while it narrows the listing, and either a toggle or a menu of values.
#[derive(IntoElement)]
pub struct FilterChip {
    id: SharedString,
    icon: &'static str,
    label: SharedString,
    lit: bool,
    kind: Kind,
}

impl FilterChip {
    pub fn toggle(
        id: impl Into<SharedString>,
        icon: &'static str,
        label: impl Into<SharedString>,
        lit: bool,
        toggle: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            icon,
            label: label.into(),
            lit,
            kind: Kind::Toggle(Rc::new(toggle)),
        }
    }

    pub fn menu(
        id: impl Into<SharedString>,
        icon: &'static str,
        label: impl Into<SharedString>,
        lit: bool,
        entries: Vec<FilterEntry>,
        pick: impl Fn(Option<String>, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            icon,
            label: label.into(),
            lit,
            kind: Kind::Menu(entries, Rc::new(pick)),
        }
    }
}

impl RenderOnce for FilterChip {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let chip = crate::Chip::new(gpui_kit::ElementId::from(self.id))
            .tone(ChipTone::Filter)
            .size(ChipSize::Medium)
            .pill(true)
            .selected(self.lit)
            .icon(self.icon)
            .label(self.label);
        match self.kind {
            Kind::Toggle(toggle) => chip.on_click(move |_, window, cx| toggle(window, cx)),
            Kind::Menu(entries, pick) => chip.dropdown(move |mut menu, _, _| {
                for entry in &entries {
                    let (value, pick) = (entry.value.clone(), pick.clone());
                    menu = menu.item(
                        PopupMenuItem::new(entry.label.clone())
                            .checked(entry.checked)
                            .on_click(move |_, window, cx| pick(value.clone(), window, cx)),
                    );
                }
                menu
            }),
        }
    }
}

/// A search field and its chips on one wrapping row.
#[derive(IntoElement)]
pub struct FilterBar {
    search: Entity<InputState>,
    chips: Vec<AnyElement>,
    narrowed: bool,
    on_clear: Option<OnClear>,
}

impl FilterBar {
    pub fn new(search: &Entity<InputState>) -> Self {
        Self {
            search: search.clone(),
            chips: Vec::new(),
            narrowed: false,
            on_clear: None,
        }
    }

    pub fn chip(mut self, chip: FilterChip) -> Self {
        self.chips.push(chip.into_any_element());
        self
    }

    /// Whether anything narrows the listing, which is when Clear is offered.
    pub fn narrowed(mut self, narrowed: bool) -> Self {
        self.narrowed = narrowed;
        self
    }

    pub fn on_clear(
        mut self,
        clear: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_clear = Some(Rc::new(clear));
        self
    }
}

impl RenderOnce for FilterBar {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let clear = self.on_clear.filter(|_| self.narrowed);
        h_flex()
            .w_full()
            .gap_2()
            .items_center()
            .flex_wrap()
            .child(
                // `max-w-2xl`: the filter never runs the width of the page.
                div()
                    .w_full()
                    .max_w(rems(26.))
                    .child(SearchField::new(&self.search).cleanable(true)),
            )
            .children(self.chips)
            .children(clear.map(|clear| {
                Button::new("filter-clear")
                    .ghost()
                    .small()
                    .icon(icons::icon(icons::CLOSE))
                    .label(t("clear"))
                    .on_click(move |event, window, cx| clear(event, window, cx))
            }))
    }
}
