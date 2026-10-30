//! Offline mode: the bar's sign that it is on, and the menu entry that turns it on or off.

use components::format::t;
use components::{Chip, ChipTone, icons};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::{AnyElement, App, IntoElement};
use store::SourcesModel;

fn is_on(cx: &App) -> bool {
    SourcesModel::global(cx)
        .read(cx)
        .network()
        .offline_mode
        .is_on()
}

/// The pill in the bar while offline mode is on; it opens the way out.
pub fn indicator(compact: bool, cx: &App) -> Option<AnyElement> {
    if !is_on(cx) {
        return None;
    }
    let chip = Chip::new("offline-mode-indicator")
        .icon(icons::CLOUD_OFF)
        .tone(ChipTone::Ink(cx.theme().primary))
        .pill(true)
        .dropdown(|menu, _, cx| item(menu.label(t("offline_mode_on")), cx));
    let chip = match compact {
        true => chip,
        false => chip.label(t("offline_mode")),
    };
    Some(chip.into_any_element())
}

/// The entry that turns offline mode on or off, checked while it is on.
pub fn item(menu: PopupMenu, cx: &App) -> PopupMenu {
    let on = is_on(cx);
    menu.item(
        PopupMenuItem::new(t("offline_mode"))
            .checked(on)
            .on_click(move |_, _, cx| {
                SourcesModel::global(cx).update(cx, |sources, cx| sources.set_offline(!on, cx));
            }),
    )
}
