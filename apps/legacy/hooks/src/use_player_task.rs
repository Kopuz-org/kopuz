//! Android: keep the UI runtime turning while the activity is hidden.
//!
//! Media buttons, the notification's shuffle and repeat, and Android Auto are
//! answered by the daemon core on its own runtime (`daemon::os_media`,
//! `daemon::android_auto`), so they never wait on this loop.

use crate::use_player_controller::PlayerController;
#[cfg(target_os = "android")]
use dioxus::prelude::*;

#[cfg(target_os = "android")]
static UI_WAKE: std::sync::OnceLock<tokio::sync::Notify> = std::sync::OnceLock::new();

pub fn use_player_task(_ctrl: PlayerController) {
    #[cfg(target_os = "android")]
    {
        use_hook(|| {
            UI_WAKE.get_or_init(tokio::sync::Notify::new);
            player::systemint::capture_event_loop();
            player::systemint::set_tokio_waker(|| {
                if let Some(notify) = UI_WAKE.get() {
                    notify.notify_one();
                }
            });
        });
        // Dioxus only advances its tasks while something here is pending, and
        // the keepalive ticker pokes this one so the UI's own work (progress,
        // queue view) keeps up in the background.
        use_future(|| async {
            loop {
                let notified = async {
                    if let Some(notify) = UI_WAKE.get() {
                        notify.notified().await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                };
                tokio::select! {
                    _ = notified => {}
                    _ = tokio::time::sleep(std::time::Duration::from_millis(250)) => {}
                }
            }
        });
    }
}
