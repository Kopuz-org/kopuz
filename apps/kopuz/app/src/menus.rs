//! The macOS menu bar, without which a full-screen window shows an empty strip and Cmd-Q does nothing.

use components::format::{t, t_with};
use gpui_kit::component::input::{Copy, Cut, Paste, Redo, SelectAll, Undo};
use gpui_kit::{App, KeyBinding, Menu, MenuItem, OsAction, SystemMenuType, Window, actions};

actions!(
    kopuz_menu,
    [
        Quit,
        Hide,
        HideOthers,
        ShowAll,
        Minimize,
        Zoom,
        ToggleFullScreen
    ]
);

/// Bind the standard macOS keystrokes and install the menus; after the keymap, so items show their keys.
pub fn init(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &Minimize, cx| on_window(cx, Window::minimize_window));
    cx.on_action(|_: &Zoom, cx| on_window(cx, Window::zoom_window));
    cx.on_action(|_: &ToggleFullScreen, cx| on_window(cx, Window::toggle_fullscreen));
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("cmd-m", Minimize, None),
        KeyBinding::new("ctrl-cmd-f", ToggleFullScreen, None),
    ]);

    let app = config::flavor::Flavor::current().display_name();
    let mut app_items = Vec::new();
    if let Some(action) = commands::get("app.settings").and_then(|command| command.action) {
        app_items.push(MenuItem::Action {
            name: t("settings"),
            action: action(),
            os_action: None,
            checked: false,
            disabled: false,
        });
        app_items.push(MenuItem::separator());
    }
    app_items.extend([
        MenuItem::os_submenu(t("menu_services"), SystemMenuType::Services),
        MenuItem::separator(),
        MenuItem::action(t_with("menu_hide", &[("app", app.into())]), Hide),
        MenuItem::action(t("menu_hide_others"), HideOthers),
        MenuItem::action(t("menu_show_all"), ShowAll),
        MenuItem::separator(),
        MenuItem::action(t_with("menu_quit", &[("app", app.into())]), Quit),
    ]);
    cx.set_menus([
        Menu::new(app).items(app_items),
        Menu::new(t("edit")).items([
            MenuItem::os_action(t("undo"), Undo, OsAction::Undo),
            MenuItem::os_action(t("redo"), Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action(t("cut"), Cut, OsAction::Cut),
            MenuItem::os_action(t("copy"), Copy, OsAction::Copy),
            MenuItem::os_action(t("paste"), Paste, OsAction::Paste),
            MenuItem::os_action(t("select_all"), SelectAll, OsAction::SelectAll),
        ]),
        Menu::new(t("view")).items([MenuItem::action(
            t("menu_toggle_full_screen"),
            ToggleFullScreen,
        )]),
        Menu::new(t("menu_window")).items([
            MenuItem::action(t("minimize"), Minimize),
            MenuItem::action(t("menu_zoom"), Zoom),
        ]),
    ]);
}

fn on_window(cx: &mut App, act: fn(&Window)) {
    let Some(window) = cx.active_window() else {
        return;
    };
    if let Err(error) = window.update(cx, |_, window, _| act(window)) {
        tracing::warn!(%error, "a menu item could not reach the window");
    }
}
