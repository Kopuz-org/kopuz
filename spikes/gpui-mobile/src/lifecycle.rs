//! Surface loss and recreation. gpui-pre-mobile handles `TerminateWindow` /
//! `InitWindow` inside its own event loop (it owns the android-activity poll)
//! and fires no GPUI callback for them, so the spike watches the state those
//! events change: whether android-activity holds a native window, and the
//! platform window's active flag. Pause/resume/resize come from GPUI
//! observers in `ui.rs`.
//!
//! Black screen after resume: gpui-pre-mobile renders a frame only for a
//! dirty window, and replacing the surface (same pixel size) does not dirty
//! it, so nothing reached the new surface until the next input. When the
//! poller sees the surface come back or the window turn active, it calls
//! `window.refresh()` through the GPUI window handle, and again on the next
//! few ticks, in case the first frame went to a surface still being swapped.
//!
//! Deadlock rule (gpui-pre-mobile 0.1.0): `AndroidPlatform::flush_main_thread_tasks`
//! runs every GPUI foreground task while holding the platform `state` mutex
//! (a non-reentrant `parking_lot::Mutex`). Any `AndroidPlatform` method that
//! locks `state` (`primary_window()`, `primary_display()`, ...) therefore
//! deadlocks the GPUI thread when called from a `cx.spawn` task. So the
//! platform window is looked up once, outside any task, and the timer only
//! touches lock-free state (`AndroidWindow::is_active` is an atomic). The
//! refresh goes through `AnyWindowHandle::update`, which only touches GPUI's
//! own window state (`Window::refresh` sets the dirty flag), never the
//! platform's; the UI's message pump updates entities the same way.

use std::sync::Arc;
use std::time::Duration;

use gpui::{AnyWindowHandle, App};
use gpui_mobile::android::jni;
use gpui_mobile::android::window::AndroidWindow;

const TICK: Duration = Duration::from_millis(250);
/// Refreshes per trigger: the tick that saw it and the next three (750 ms).
const REFRESH_TICKS: u8 = 4;

fn surface_present() -> bool {
    jni::android_app().is_some_and(|app| app.native_window().is_some())
}

/// Not callable from a foreground task (see the module docs).
fn primary_window() -> Option<Arc<AndroidWindow>> {
    jni::platform().and_then(|platform| platform.primary_window())
}

/// Logical px of the status bar, from the content rect the platform tracks.
/// Not callable from a foreground task (see the module docs).
pub fn top_inset() -> f32 {
    primary_window()
        .map(|window| window.safe_area_insets_logical().top)
        .unwrap_or(0.0)
}

/// The platform window, looked up while no platform lock is held.
pub struct Watcher {
    window: Option<Arc<AndroidWindow>>,
}

impl Watcher {
    /// Call from the `Application::run` callback, not from a task: the window
    /// lookup locks the platform state. gpui-pre-mobile keeps the same
    /// `AndroidWindow` across surface loss (it re-inits it), so it stays valid.
    pub fn new() -> Self {
        let window = primary_window();
        if window.is_none() {
            log::info!("SPIKE error lifecycle: no platform window");
        }
        Self { window }
    }

    /// Poll the surface and active state, and refresh `handle` after a
    /// surface comes back or the window turns active.
    pub fn spawn(self, cx: &mut App, handle: AnyWindowHandle) {
        let window = self.window;
        let window_active = move || window.as_ref().is_some_and(|window| window.is_active());
        cx.spawn(async move |cx| {
            let mut surface = surface_present();
            let mut active = window_active();
            // (reason, ticks left, tick number)
            let mut refresh: Option<(&'static str, u8, u8)> = None;
            loop {
                cx.background_executor().timer(TICK).await;
                let now_surface = surface_present();
                if now_surface != surface {
                    let event = if now_surface {
                        "surface_recreated"
                    } else {
                        "surface_lost"
                    };
                    log::info!("SPIKE lifecycle {event}");
                    surface = now_surface;
                    if now_surface {
                        refresh = Some((event, REFRESH_TICKS, 0));
                    }
                }
                let now_active = window_active();
                if now_active != active {
                    let event = if now_active {
                        "platform_active"
                    } else {
                        "platform_inactive"
                    };
                    log::info!("SPIKE lifecycle {event}");
                    active = now_active;
                    // A surface that just came back names the cause; keep it.
                    if now_active && refresh.is_none() {
                        refresh = Some((event, REFRESH_TICKS, 0));
                    }
                }
                if let Some((reason, left, tick)) = refresh {
                    let tick = tick + 1;
                    match handle.update(cx, |_, window, _| window.refresh()) {
                        Ok(()) => log::info!("SPIKE refresh reason={reason} tick={tick}"),
                        Err(error) => log::info!("SPIKE error refresh reason={reason}: {error}"),
                    }
                    refresh = (left > 1).then_some((reason, left - 1, tick));
                }
            }
        })
        .detach();
    }
}
