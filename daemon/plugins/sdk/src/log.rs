//! Log into the host's log, tagged with the plugin's id.

pub use crate::bindings::kopuz::plugin::log::Level;

/// Outside wasm there is no host, so native unit tests drop the message.
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
