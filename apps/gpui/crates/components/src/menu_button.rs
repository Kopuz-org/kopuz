//! The dots button every row, card and pill opens its menu from, keeping its own open state.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::component::{ActiveTheme as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, Pixels, RenderOnce,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
};

use crate::icon_button::{IconButton, IconTone};
use crate::menus::Menu;
use crate::{focus, icons, motion, style};

const OPEN: u64 = 20;
const BUTTON: u64 = 21;
const SLOT: u64 = 22;
const FADE: u64 = 23;

/// How the dots sit on what holds them.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum MenuVariant {
    /// Washes under the pointer and paints nothing otherwise: a table row's end cell.
    #[default]
    Ghost,
    /// A dark round chip, for dots floating over art or a raised card.
    Scrim,
    /// Paints nothing at all and keeps `ink`, for a slab that owns the colour.
    Bare(Hsla),
    /// An [`IconButton`] of the given look, for dots that sit in a row of round controls.
    Icon {
        size: Pixels,
        glyph: Pixels,
        tone: IconTone,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
enum Reveal {
    #[default]
    Always,
    Fade(bool),
    Group(&'static str),
}

/// A dots button opening `menu`; pressing it is never a click on what holds it.
#[derive(IntoElement)]
pub struct MenuButton {
    id: SharedString,
    menu: Menu,
    variant: MenuVariant,
    reveal: Reveal,
}

impl MenuButton {
    pub fn new(id: impl Into<SharedString>, menu: Menu) -> Self {
        Self {
            id: id.into(),
            menu,
            variant: MenuVariant::default(),
            reveal: Reveal::Always,
        }
    }

    pub fn variant(mut self, variant: MenuVariant) -> Self {
        self.variant = variant;
        self
    }

    /// Fades in while `shown`, and stays shown for as long as its own menu is open.
    pub fn reveal(mut self, shown: bool) -> Self {
        self.reveal = Reveal::Fade(shown);
        self
    }

    /// Shown while the pointer is over the named hover group, or its own menu is open.
    pub fn reveal_in_group(mut self, group: &'static str) -> Self {
        self.reveal = Reveal::Group(group);
        self
    }

    /// Whether the menu of the button with this id is open, for a parent that wants to stay lit.
    pub fn is_open(id: &SharedString, window: &mut Window, cx: &mut App) -> bool {
        *window
            .use_keyed_state(motion::channel(id, OPEN), cx, |_, _| false)
            .read(cx)
    }
}

impl RenderOnce for MenuButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self.id;
        let state = window.use_keyed_state(motion::channel(&id, OPEN), cx, |_, _| false);
        let open = *state.read(cx);
        let menu = self.menu;
        let on_open = move |shown: &bool, _: &mut Window, cx: &mut App| {
            let shown = *shown;
            state.update(cx, |open, cx| {
                *open = shown;
                cx.notify();
            });
        };
        let trigger = motion::channel(&id, BUTTON);
        let dots = match self.variant {
            MenuVariant::Icon { size, glyph, tone } => IconButton::new(id.clone(), icons::MORE)
                .size(size)
                .glyph(glyph)
                .tone(tone)
                .host(move |button| {
                    button
                        .dropdown_menu(move |popup, window, cx| menu(popup, window, cx))
                        .on_open_change(on_open)
                        .into_any_element()
                })
                .into_any_element(),
            variant => {
                let button = Button::new(trigger).icon(icons::icon(icons::MORE));
                let button = match variant {
                    MenuVariant::Ghost | MenuVariant::Icon { .. } => button.ghost().xsmall(),
                    MenuVariant::Scrim => button
                        .custom(style::button(
                            style::black(0.4, cx),
                            style::white(style::HOVER, cx),
                            cx.theme().secondary_foreground,
                            cx,
                        ))
                        .rounded(cx.theme().radius_full()),
                    MenuVariant::Bare(ink) => button.custom(style::bare_button(ink, cx)),
                };
                button
                    .dropdown_menu(move |popup, window, cx| menu(popup, window, cx))
                    .on_open_change(on_open)
                    .into_any_element()
            }
        };
        let slot = div()
            .id(motion::channel(&id, SLOT))
            .on_click(|_, _, cx| cx.stop_propagation());
        match self.reveal {
            Reveal::Always => slot.child(dots),
            // A hidden trigger would take its own open menu down with it, so an open one stays.
            Reveal::Group(group) => slot
                .when(!open, |slot| {
                    slot.invisible().group_hover(group, |dots| dots.visible())
                })
                .child(dots),
            Reveal::Fade(shown) => {
                let amount = motion::amount(
                    motion::channel(&id, FADE),
                    focus::reveal(shown || open),
                    motion::FAST,
                    window,
                    cx,
                );
                slot.opacity(amount).child(dots)
            }
        }
    }
}
