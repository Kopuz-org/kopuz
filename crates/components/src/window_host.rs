//! The single renderer seam for host-window operations.
//!
//! Window controls (decorations, drag, min/max/close, visibility, resize) are
//! host-shell operations with no HTML/CSS surface, so each renderer exposes its
//! own handle: the webview a wry/tao `DesktopContext`, the native renderer a
//! blitz `ShellProvider` (close + chrome) plus the winit `Window` (everything
//! else). Every renderer divergence for window control is resolved here.

#[cfg(not(target_arch = "wasm32"))]
use dioxus::prelude::*;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use winit::window::{Window, WindowLevel};

#[cfg(not(target_arch = "wasm32"))]
fn desktop() -> Option<dioxus::desktop::DesktopContext> {
    try_consume_context::<dioxus::desktop::DesktopContext>()
}

#[cfg(not(target_arch = "wasm32"))]
fn shell() -> Option<Arc<dyn blitz_traits::shell::ShellProvider>> {
    try_consume_context::<Arc<dyn blitz_traits::shell::ShellProvider>>()
}

#[cfg(not(target_arch = "wasm32"))]
fn winit_window() -> Option<Arc<dyn Window>> {
    try_consume_context::<Arc<dyn Window>>()
}

/// Whether a host window is available (a desktop renderer is hosting us).
pub fn available() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        false
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        desktop().is_some() || winit_window().is_some()
    }
}

/// True when a webview (wry/tao) host is active. The webview pumps the global
/// muda / tray-icon event channels through its event loop; the native renderer
/// (winit) does not, so callers must poll those channels themselves.
pub fn is_webview() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        false
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        desktop().is_some()
    }
}

pub fn set_decorations(decorations: bool) {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(w) = desktop() {
        w.set_decorations(decorations);
    } else if let Some(s) = shell() {
        s.set_window_decorations(decorations);
    }
    #[cfg(target_arch = "wasm32")]
    let _ = decorations;
}

/// Begin an interactive window move (from a mousedown handler on a drag region).
pub fn drag() {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(w) = desktop() {
        w.drag();
    } else if let Some(s) = shell() {
        s.drag_window();
    }
}

pub fn minimize() {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(w) = desktop() {
        w.window.set_minimized(true);
    } else if let Some(s) = shell() {
        s.set_window_minimized(true);
    }
}

pub fn toggle_maximized() {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(w) = desktop() {
        w.toggle_maximized();
    } else if let Some(s) = shell() {
        s.set_window_maximized(!s.is_window_maximized());
    }
}

pub fn close() {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(w) = desktop() {
        w.close();
    } else if let Some(s) = shell() {
        s.request_window_close();
    }
}

pub fn is_visible() -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Some(w) = desktop() {
            return w.is_visible();
        }
        if let Some(win) = winit_window() {
            return win.is_visible().unwrap_or(true);
        }
    }
    true
}

pub fn set_visible(visible: bool) {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(w) = desktop() {
        w.set_visible(visible);
    } else if let Some(win) = winit_window() {
        win.set_visible(visible);
    }
    #[cfg(target_arch = "wasm32")]
    let _ = visible;
}

pub fn set_focus() {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(w) = desktop() {
        w.set_focus();
    } else if let Some(win) = winit_window() {
        win.focus_window();
    }
}

/// Whether closing the window hides it (kept alive in the tray) instead of
/// quitting. Webview-only for now; under the native renderer this is a no-op
/// (blitz exits on close — hide-on-close needs a blitz-shell lifecycle hook).
pub fn set_hide_on_close(hide: bool) {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(w) = desktop() {
        use dioxus::desktop::WindowCloseBehaviour::{WindowCloses, WindowHides};
        w.set_close_behavior(if hide { WindowHides } else { WindowCloses });
    }
    #[cfg(target_arch = "wasm32")]
    let _ = hide;
}

pub fn scale_factor() -> f64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Some(w) = desktop() {
            return w.window.scale_factor();
        }
        if let Some(win) = winit_window() {
            return win.scale_factor();
        }
    }
    1.0
}

/// Current window inner size in logical pixels.
pub fn inner_size_logical() -> (f64, f64) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Some(w) = desktop() {
            let scale = w.window.scale_factor();
            let s = w.window.inner_size().to_logical::<f64>(scale);
            return (s.width, s.height);
        }
        if let Some(win) = winit_window() {
            let scale = win.scale_factor();
            let s = win.surface_size().to_logical::<f64>(scale);
            return (s.width, s.height);
        }
    }
    (0.0, 0.0)
}

pub fn set_always_on_top(on: bool) {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(w) = desktop() {
        w.window.set_always_on_top(on);
    } else if let Some(win) = winit_window() {
        win.set_window_level(if on {
            WindowLevel::AlwaysOnTop
        } else {
            WindowLevel::Normal
        });
    }
    #[cfg(target_arch = "wasm32")]
    let _ = on;
}

pub fn set_resizable(resizable: bool) {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(w) = desktop() {
        w.window.set_resizable(resizable);
    } else if let Some(win) = winit_window() {
        win.set_resizable(resizable);
    }
    #[cfg(target_arch = "wasm32")]
    let _ = resizable;
}

pub fn set_min_inner_size(size: Option<(f64, f64)>) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let logical = size.map(|(w, h)| winit::dpi::LogicalSize::new(w, h));
        if let Some(w) = desktop() {
            w.window
                .set_min_inner_size(logical.map(winit::dpi::Size::Logical));
        } else if let Some(win) = winit_window() {
            win.set_min_surface_size(logical.map(winit::dpi::Size::Logical));
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = size;
}

pub fn set_max_inner_size(size: Option<(f64, f64)>) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let logical = size.map(|(w, h)| winit::dpi::LogicalSize::new(w, h));
        if let Some(w) = desktop() {
            w.window
                .set_max_inner_size(logical.map(winit::dpi::Size::Logical));
        } else if let Some(win) = winit_window() {
            win.set_max_surface_size(logical.map(winit::dpi::Size::Logical));
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = size;
}

pub fn set_inner_size(width: f64, height: f64) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let logical = winit::dpi::LogicalSize::new(width, height);
        if let Some(w) = desktop() {
            let _ = w.window.set_inner_size(logical);
        } else if let Some(win) = winit_window() {
            let _ = win.request_surface_size(winit::dpi::Size::Logical(logical));
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (width, height);
}
