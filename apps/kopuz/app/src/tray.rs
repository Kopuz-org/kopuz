//! The tray icon `system.minimize_to_tray` asks for: Show / Hide and Quit, and a closed window leaves Kopuz running.

use std::time::Instant;

use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui_kit::{App, Global, QuitMode, Task};
use store::ConfigModel;
use tray_icon::menu::{ContextMenu, Menu, MenuEvent, MenuId, MenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use components::format::t;

const SHOW_HIDE: &str = "kopuz-tray-show";
const QUIT: &str = "kopuz-tray-quit";

/// The icon while it is up, and the language its menu was labelled in.
#[derive(Default)]
struct Tray {
    icon: Option<(TrayIcon, String)>,
}

impl Global for Tray {}

/// The menu's clicks, which arrive on whatever thread the platform delivers them on.
struct Clicks {
    _listen: Task<()>,
}

impl Global for Clicks {}

/// Follow the setting from now on; called once the main window is up.
pub fn init(cx: &mut App) {
    let (sender, mut clicks) = mpsc::unbounded::<MenuId>();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        sender.unbounded_send(event.id).ok();
    }));
    let listen = cx.spawn(async move |cx| {
        while let Some(id) = clicks.next().await {
            cx.update(|cx| clicked(&id, cx));
        }
    });
    cx.set_global(Clicks { _listen: listen });
    cx.set_global(Tray::default());
    sync(cx);
    cx.observe(&ConfigModel::global(cx), |_, cx| sync(cx))
        .detach();
}

fn clicked(id: &MenuId, cx: &mut App) {
    match id.as_ref() {
        SHOW_HIDE => match crate::main_window(cx) {
            Some(window) => {
                if let Err(error) = window.update(cx, |_, window, _| window.remove_window()) {
                    tracing::warn!(%error, "could not hide the window to the tray");
                }
            }
            None => {
                crate::open_main_window(Instant::now(), None, cx);
                cx.activate(true);
            }
        },
        QUIT => cx.quit(),
        _ => {}
    }
}

/// Put the icon up or take it down as the setting says, and relabel it when the language changes.
fn sync(cx: &mut App) {
    let wanted = ConfigModel::global(cx)
        .read(cx)
        .config()
        .system
        .minimize_to_tray;
    let language = ConfigModel::global(cx)
        .read(cx)
        .config()
        .ui
        .language
        .clone();
    let mut tray = cx.remove_global::<Tray>();
    let mut taken_down = false;
    match (wanted, tray.icon.as_mut()) {
        (false, Some(_)) => {
            tray.icon = None;
            taken_down = true;
        }
        (true, None) => tray.icon = build().map(|icon| (icon, language)),
        (true, Some((icon, labelled))) if *labelled != language => {
            icon.set_menu(menu().map(|menu| Box::new(menu) as Box<dyn ContextMenu>));
            *labelled = language;
        }
        _ => {}
    }
    // Closing the window only hides Kopuz while the tray is there to bring it back.
    cx.set_quit_mode(match tray.icon.is_some() {
        true => QuitMode::Explicit,
        false => QuitMode::Default,
    });
    cx.set_global(tray);
    // With the icon gone, nothing else could bring a hidden window back.
    if taken_down && crate::main_window(cx).is_none() {
        crate::open_main_window(Instant::now(), None, cx);
    }
}

fn build() -> Option<TrayIcon> {
    let mut builder = TrayIconBuilder::new()
        .with_tooltip(config::flavor::Flavor::current().display_name())
        .with_menu(Box::new(menu()?));
    if let Some(icon) = icon() {
        builder = builder.with_icon(icon);
    }
    match builder.build() {
        Ok(tray) => Some(tray),
        Err(error) => {
            tracing::warn!(%error, "could not put the tray icon up");
            None
        }
    }
}

fn menu() -> Option<Menu> {
    let show = MenuItem::with_id(SHOW_HIDE, t("tray_show_hide"), true, None);
    let quit = MenuItem::with_id(QUIT, t("tray_quit"), true, None);
    match Menu::with_items(&[&show, &quit]) {
        Ok(menu) => Some(menu),
        Err(error) => {
            tracing::warn!(%error, "could not build the tray menu");
            None
        }
    }
}

fn icon() -> Option<Icon> {
    let image = match image::load_from_memory(include_bytes!(
        "../../../legacy/kopuz/assets/logo-128.png"
    )) {
        Ok(image) => image.into_rgba8(),
        Err(error) => {
            tracing::warn!(%error, "could not read the tray icon");
            return None;
        }
    };
    let (width, height) = image.dimensions();
    Icon::from_rgba(image.into_raw(), width, height)
        .inspect_err(|error| tracing::warn!(%error, "could not make the tray icon"))
        .ok()
}
