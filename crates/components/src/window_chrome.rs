//! The single renderer seam for window chrome.
//!
//! Window controls (drag/minimize/maximize/close/decorations) are host-shell
//! operations with no standard HTML/CSS surface, so each renderer exposes its
//! own handle: the webview provides a wry/tao `DesktopContext`, the native
//! renderer a blitz `ShellProvider`. Everything outside this module is
//! renderer-agnostic — the divergence is resolved here and nowhere else.

#[cfg(not(target_arch = "wasm32"))]
use dioxus::prelude::*;

#[cfg(not(target_arch = "wasm32"))]
fn desktop_window() -> Option<dioxus::desktop::DesktopContext> {
    try_consume_context::<dioxus::desktop::DesktopContext>()
}

#[cfg(not(target_arch = "wasm32"))]
fn shell_provider() -> Option<std::sync::Arc<dyn blitz_traits::shell::ShellProvider>> {
    try_consume_context::<std::sync::Arc<dyn blitz_traits::shell::ShellProvider>>()
}

/// Whether a window handle is available (a desktop renderer is hosting us).
pub fn available() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        false
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        desktop_window().is_some() || shell_provider().is_some()
    }
}

/// Begin an interactive user-driven window move (call from a mousedown
/// handler on a drag region).
pub fn drag() {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(w) = desktop_window() {
        w.drag();
    } else if let Some(s) = shell_provider() {
        s.drag_window();
    }
}

pub fn minimize() {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(w) = desktop_window() {
        w.window.set_minimized(true);
    } else if let Some(s) = shell_provider() {
        s.set_window_minimized(true);
    }
}

pub fn toggle_maximized() {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(w) = desktop_window() {
        w.toggle_maximized();
    } else if let Some(s) = shell_provider() {
        s.set_window_maximized(!s.is_window_maximized());
    }
}

pub fn close() {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(w) = desktop_window() {
        w.close();
    } else if let Some(s) = shell_provider() {
        s.request_window_close();
    }
}

pub fn set_decorations(decorations: bool) {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(w) = desktop_window() {
        w.set_decorations(decorations);
    } else if let Some(s) = shell_provider() {
        s.set_window_decorations(decorations);
    }
    #[cfg(target_arch = "wasm32")]
    let _ = decorations;
}
