#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
pub use macos::{
    SystemEvent, init, park_main_loop, refresh_now_playing, set_background_handler,
    set_tokio_waker, update_now_playing, wake_run_loop,
};

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
pub use linux::{
    RepeatMode, SystemEvent, poll_event, update_modes, update_now_playing, update_position,
    wait_event,
};

#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "windows")]
pub use windows::{SystemEvent, attach_window, init, poll_event, update_now_playing, wait_event};

#[cfg(target_os = "android")]
mod android;

#[cfg(target_os = "android")]
pub use android::{
    RepeatMode, SystemEvent, await_media_permission, capture_event_loop, get_android_music_dir,
    get_files_dir, has_media_permission, init, login_close, login_cookies, login_is_open,
    login_open, move_task_to_back, request_permissions, set_background_handler, set_keepalive,
    set_tokio_waker, stop_session, take_back_pressed, update_modes, update_now_playing,
    wait_back_pressed, wake_run_loop,
};

#[cfg(not(target_os = "android"))]
pub fn request_permissions() {}

#[cfg(not(target_os = "android"))]
pub fn get_android_music_dir() -> Option<String> {
    None
}

// notify_one stores a permit, so a back press between checking the pending flag
// and awaiting the notification still wakes the UI task.
use std::sync::OnceLock;
use tokio::sync::Notify;

fn back_notify() -> &'static Notify {
    static N: OnceLock<Notify> = OnceLock::new();
    N.get_or_init(Notify::new)
}

/// Wake the Android back-handling loop now. Sync, any thread.
pub fn back_wake() {
    back_notify().notify_one();
}
