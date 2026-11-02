//! The one menu builder, reached from below: `commands` registers it, every row and card calls it.

use std::cell::Cell;
use std::rc::Rc;

use api::{ArtworkRef, ArtworkTarget};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenu, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::{
    AnyElement, App, Bounds, ClickEvent, Context, Global, IntoElement, ParentElement as _, Pixels,
    SharedString, Styled as _, Window,
};
use store::{Pictured, Target};

use crate::format::t;
use crate::icons;

type Builder = Rc<dyn Fn(Target, PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu>;

#[derive(Default)]
struct Menus {
    build: Option<Builder>,
}

impl Global for Menus {}

/// Install the registry's builder; `commands::init` is the only caller.
pub fn register(
    builder: impl Fn(Target, PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    cx: &mut App,
) {
    cx.set_global(Menus {
        build: Some(Rc::new(builder)),
    });
}

/// Fill `menu` with what applies to `target`; an unregistered builder leaves it untouched.
pub fn build(
    target: Target,
    menu: PopupMenu,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let builder = cx
        .try_global::<Menus>()
        .and_then(|menus| menus.build.clone());
    match builder {
        Some(builder) => builder(target, menu, window, cx),
        None => {
            tracing::warn!("no menu builder is registered");
            menu
        }
    }
}

/// The closure `dropdown_menu` and `context_menu` both take.
pub fn builder(
    target: Target,
) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + Clone + 'static {
    move |menu, window, cx| build(target.clone(), menu, window, cx)
}

/// One builder shared by a dots button and a right-click, so the two menus cannot drift apart.
pub type Menu = Rc<dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu>;

/// The menu for `target`, ready to hand to a [`crate::MenuButton`] or [`WithContextMenu`].
pub fn menu_for(target: Target) -> Menu {
    Rc::new(builder(target))
}

/// Right-click opens `menu`, when there is one.
pub trait WithContextMenu: ContextMenuExt + IntoElement + Sized + 'static {
    fn with_context_menu(self, menu: Option<Menu>) -> AnyElement {
        match menu {
            Some(menu) => self
                .context_menu(move |popup, window, cx| menu(popup, window, cx))
                .into_any_element(),
            None => self.into_any_element(),
        }
    }
}

impl<T: ContextMenuExt + IntoElement + Sized + 'static> WithContextMenu for T {}

/// Where a thumbnail was drawn this frame, so its menu can tell a right-click on it from one beside it.
#[derive(Clone, Default)]
pub struct Spot(Rc<SpotState>);

#[derive(Default)]
struct SpotState {
    drawn: Cell<Option<Bounds<Pixels>>>,
    broken: Cell<bool>,
}

impl Spot {
    pub(crate) fn drawn(&self, bounds: Bounds<Pixels>) {
        self.0.drawn.set(Some(bounds));
    }

    /// The picture would not load, so the thumbnail is a placeholder after all.
    pub(crate) fn broken(&self) {
        self.0.broken.set(true);
    }

    fn under(&self, window: &Window) -> bool {
        !self.0.broken.get()
            && self
                .0
                .drawn
                .get()
                .is_some_and(|bounds| bounds.contains(&window.mouse_position()))
    }
}

/// A thumbnail's picture as its menu offers it: where it is drawn, what to fetch, and what it is of.
#[derive(Clone)]
pub struct Picture {
    spot: Spot,
    target: ArtworkTarget,
    of: Pictured,
}

impl Picture {
    /// Nothing for a placeholder: only real artwork can be copied or saved.
    pub fn new(artwork: Option<&ArtworkRef>, of: Pictured) -> Option<Self> {
        artwork.map(|artwork| Self {
            spot: Spot::default(),
            target: artwork.target.clone(),
            of,
        })
    }

    /// What the thumbnail records itself into; hand it to its [`crate::Cover`].
    pub fn spot(&self) -> Spot {
        self.spot.clone()
    }
}

/// `menu`, ending in Copy image and Save image when the right-click landed on `picture`'s thumbnail.
pub fn pictured(menu: Option<Menu>, picture: Option<Picture>) -> Option<Menu> {
    let menu = menu?;
    let Some(picture) = picture else {
        return Some(menu);
    };
    Some(Rc::new(move |popup, window, cx| {
        let popup = menu(popup, window, cx);
        match picture.spot.under(window) {
            true => picture_items(popup, &picture),
            false => popup,
        }
    }))
}

fn picture_items(popup: PopupMenu, picture: &Picture) -> PopupMenu {
    let (copy, save, of) = (
        picture.target.clone(),
        picture.target.clone(),
        picture.of.clone(),
    );
    popup
        .separator()
        .item(
            menu_item(t("copy_image"), Some(icons::COPY))
                .on_click(move |_, _, cx| store::copy_picture(copy.clone(), cx)),
        )
        .item(
            menu_item(t("save_image"), Some(icons::DOWNLOAD))
                .on_click(move |_, _, cx| store::save_picture(save.clone(), of.clone(), cx)),
        )
}

type OnPick = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// One menu entry: a label, an optional glyph, and a danger colour for what takes something away.
pub struct MenuItem {
    label: SharedString,
    icon: Option<&'static str>,
    danger: bool,
    pick: Option<OnPick>,
}

pub fn menu_item(label: impl Into<SharedString>, icon: Option<&'static str>) -> MenuItem {
    MenuItem {
        label: label.into(),
        icon,
        danger: false,
        pick: None,
    }
}

impl MenuItem {
    pub fn danger(mut self, danger: bool) -> Self {
        self.danger = danger;
        self
    }

    pub fn on_click(mut self, pick: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.pick = Some(Rc::new(pick));
        self
    }
}

impl From<MenuItem> for PopupMenuItem {
    fn from(item: MenuItem) -> Self {
        let MenuItem {
            label,
            icon,
            danger,
            pick,
        } = item;
        let entry = if danger {
            PopupMenuItem::element(move |_, cx| {
                h_flex()
                    .gap_2()
                    .items_center()
                    .text_color(cx.theme().danger)
                    .children(icon.map(crate::icons::icon))
                    .child(label.clone())
                    .into_any_element()
            })
        } else {
            let entry = PopupMenuItem::new(label);
            match icon {
                Some(icon) => entry.icon(crate::icons::icon(icon)),
                None => entry,
            }
        };
        match pick {
            Some(pick) => entry.on_click(move |event, window, cx| pick(event, window, cx)),
            None => entry,
        }
    }
}

/// The same builder for a target that is costly to make, such as a long selection: made when the menu opens.
pub fn lazy_builder(
    target: impl Fn() -> Target + 'static,
) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
    move |menu, window, cx| build(target(), menu, window, cx)
}
