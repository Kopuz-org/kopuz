//! Log into the host's log, tagged with the plugin's id.
//!
//! ```ignore
//! kopuz_plugin_sdk::debug!("fetched {} albums", albums.len());
//! ```

pub use crate::bindings::kopuz::plugin::log::Level;

/// Send one message to the host's log. Outside wasm there is no host, so the
/// message is dropped; that keeps native unit tests of plugin code running.
pub fn write(level: Level, message: &str) {
    #[cfg(target_arch = "wasm32")]
    crate::bindings::kopuz::plugin::log::log(level, message);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (level, message);
}

#[macro_export]
macro_rules! trace {
    ($($arg:tt)*) => { $crate::log::write($crate::log::Level::Trace, &::std::format!($($arg)*)) };
}

#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => { $crate::log::write($crate::log::Level::Debug, &::std::format!($($arg)*)) };
}

#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => { $crate::log::write($crate::log::Level::Info, &::std::format!($($arg)*)) };
}

#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => { $crate::log::write($crate::log::Level::Warn, &::std::format!($($arg)*)) };
}

#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => { $crate::log::write($crate::log::Level::Error, &::std::format!($($arg)*)) };
}
