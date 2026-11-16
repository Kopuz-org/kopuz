//! Utility packages for common mobile operations.
//!
//! Each package is feature-gated and provides a shared API with
//! platform-specific implementations for iOS and Android.

#[cfg(feature = "package_info")]
pub mod package_info;

#[cfg(feature = "device_info")]
pub mod device_info;

#[cfg(feature = "path_provider")]
pub mod path_provider;

#[cfg(feature = "shared_preferences")]
pub mod shared_preferences;

#[cfg(feature = "url_launcher")]
pub mod url_launcher;

#[cfg(feature = "vibration")]
pub mod vibration;

#[cfg(feature = "connectivity")]
pub mod connectivity;

#[cfg(feature = "network_info")]
pub mod network_info;

#[cfg(feature = "battery")]
pub mod battery;

#[cfg(feature = "share")]
pub mod share;

#[cfg(feature = "sensors")]
pub mod sensors;

#[cfg(feature = "webview")]
pub mod webview;

#[cfg(feature = "file_selector")]
pub mod file_selector;

#[cfg(feature = "image_picker")]
pub mod image_picker;

#[cfg(feature = "camera")]
pub mod camera;

#[cfg(feature = "permission_handler")]
pub mod permission_handler;

#[cfg(feature = "notifications")]
pub mod notifications;

#[cfg(feature = "location")]
pub mod location;

#[cfg(feature = "audio")]
pub mod audio;

#[cfg(feature = "video_player")]
pub mod video_player;

#[cfg(feature = "media_session")]
pub mod media_session;

#[cfg(feature = "clipboard")]
pub mod clipboard;

#[cfg(feature = "in_app_review")]
pub mod in_app_review;

#[cfg(feature = "local_auth")]
pub mod local_auth;

#[cfg(feature = "contacts")]
pub mod contacts;

#[cfg(feature = "calendar")]
pub mod calendar;

#[cfg(feature = "maps_launcher")]
pub mod maps_launcher;

#[cfg(feature = "maps")]
pub mod maps;

#[cfg(feature = "deeplink")]
pub mod deeplink;

#[cfg(feature = "microphone")]
pub mod microphone;
