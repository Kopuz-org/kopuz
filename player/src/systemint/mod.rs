#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
pub use macos::{SystemEvent, init, refresh_now_playing, set_background_handler, set_tokio_waker, update_now_playing, wake_run_loop};

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
pub use linux::{SystemEvent, poll_event, update_now_playing};

#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "windows")]
pub use windows::{SystemEvent, init, poll_event, update_now_playing, wait_event};
