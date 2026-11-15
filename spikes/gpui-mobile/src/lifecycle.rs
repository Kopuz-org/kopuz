//! Surface loss and recreation. gpui-pre-mobile handles `TerminateWindow` /
//! `InitWindow` inside its own event loop (it owns the android-activity poll)
//! and fires no GPUI callback for them, so the spike watches the state those
//! events change: whether android-activity holds a native window, and the
//! platform window's active flag. Pause/resume/resize come from GPUI
//! observers in `ui.rs`.
//!
//! Deadlock rule (gpui-pre-mobile 0.1.0): `AndroidPlatform::flush_main_thread_tasks`
//! runs every GPUI foreground task while holding the platform `state` mutex
//! (a non-reentrant `parking_lot::Mutex`). Any `AndroidPlatform` method that
//! locks `state` (`primary_window()`, `primary_display()`, ...) therefore
//! deadlocks the GPUI thread when called from a `cx.spawn` task. So the
//! platform window is looked up once, outside any task, and the timer only
//! touches lock-free state (`AndroidWindow::is_active` is an atomic).

use std::sync::Arc;
use std::time::Duration;

use gpui::App;
use gpui_mobile::android::jni;
use gpui_mobile::android::window::AndroidWindow;

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

/// Call from the `Application::run` callback, not from a task: the window
/// lookup locks the platform state. gpui-pre-mobile keeps the same
/// `AndroidWindow` across surface loss (it re-inits it), so the handle stays valid.
pub fn watch(cx: &mut App) {
    let window = primary_window();
    if window.is_none() {
        log::info!("SPIKE error lifecycle: no platform window");
    }
    let window_active = move || window.as_ref().is_some_and(|window| window.is_active());
    cx.spawn(async move |cx| {
        let mut surface = surface_present();
        let mut active = window_active();
        loop {
            cx.background_executor()
                .timer(Duration::from_millis(250))
                .await;
            let now_surface = surface_present();
            if now_surface != surface {
                let event = if now_surface {
                    "surface_recreated"
                } else {
                    "surface_lost"
                };
                log::info!("SPIKE lifecycle {event}");
                surface = now_surface;
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
            }
        }
    })
    .detach();
}
