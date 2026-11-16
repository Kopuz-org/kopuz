//! Phone layout: set only by the Android host, so desktop never takes these paths.

use std::rc::Rc;

use gpui_kit::{App, Global, Pixels, px};

/// Smallest touch target on a phone (Material's 48 dp; GPUI's logical pixels are dp on Android).
pub const TOUCH_TARGET: Pixels = px(48.);

/// The phone's bottom tab bar, not counting the gesture bar's inset under it (see [`bottom_chrome`]).
pub const TAB_BAR_HEIGHT: Pixels = px(64.);

/// The phone's slim header over the page, not counting the status bar's inset above it.
pub const HEADER_HEIGHT: Pixels = px(52.);

/// The system's safe-area insets around the window, in logical pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Insets {
    pub top: Pixels,
    pub bottom: Pixels,
    pub left: Pixels,
    pub right: Pixels,
}

/// Present only when the app runs on a phone; the Android host installs it and keeps the insets current.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Mobile {
    pub insets: Insets,
    /// The soft keyboard's height from the window's bottom edge; zero while it is hidden.
    pub keyboard: Pixels,
}

impl Global for Mobile {}

/// Whether the phone layout applies: true only under the Android host.
pub fn is_compact(cx: &App) -> bool {
    cx.has_global::<Mobile>()
}

/// Whether input is touch, so keyboard-shortcut hints and hover-only affordances hide.
pub fn is_touch(cx: &App) -> bool {
    cx.has_global::<Mobile>()
}

/// The safe-area insets to keep chrome out of the status bar, cutout and gesture bar; zero on desktop.
pub fn safe_insets(cx: &App) -> Insets {
    cx.try_global::<Mobile>()
        .map(|mobile| mobile.insets)
        .unwrap_or_default()
}

/// The soft keyboard's height from the window's bottom edge; zero while hidden, and on desktop.
/// A floating keyboard reports only the gesture bar under it, which covers nothing, so it reads zero.
pub fn keyboard(cx: &App) -> Pixels {
    cx.try_global::<Mobile>()
        .map(|mobile| mobile.keyboard)
        .filter(|keyboard| *keyboard > safe_insets(cx).bottom)
        .unwrap_or_default()
}

/// What the tab bar takes from the window's bottom edge on a phone, the gesture bar's inset included;
/// zero on desktop. Whatever sits above the tab bar, or rises from the bottom, clears this much.
pub fn bottom_chrome(cx: &App) -> Pixels {
    match is_compact(cx) {
        true => TAB_BAR_HEIGHT + safe_insets(cx).bottom,
        false => px(0.),
    }
}

/// What a panel rising from the bottom keeps clear of: the tab bar, or the soft keyboard while it covers more.
pub fn bottom_clearance(cx: &App) -> Pixels {
    bottom_chrome(cx).max(keyboard(cx))
}

/// Installs or updates the phone layout global; called by the Android host only.
pub fn set_mobile(mobile: Mobile, cx: &mut App) {
    cx.set_global(mobile);
}

/// What only the host can do for the app: sending it to the background, as back does on a root screen.
#[derive(Clone)]
pub struct Host {
    pub minimize: Rc<dyn Fn()>,
}

impl Global for Host {}

/// Installs the host's services; called by the Android host only.
pub fn set_host(host: Host, cx: &mut App) {
    cx.set_global(host);
}

/// Send the app to the background; nothing happens without a host.
pub fn minimize(cx: &App) {
    if let Some(host) = cx.try_global::<Host>() {
        (host.minimize)();
    }
}
