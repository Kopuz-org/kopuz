//! Android platform implementation for GPUI.
//!
//! This module wires together all Android-specific sub-modules and exposes the
//! single public entry-point `current_platform()`, mirroring the structure of
//! `gpui_linux::linux`.
//!
//! ## Architecture
//!
//! ```text
//! AndroidPlatform             (platform.rs)
//!   ├── AndroidDispatcher     (dispatcher.rs)
//!   ├── AndroidWindow         (window.rs)
//!   │     └── gpui_wgpu::WgpuRenderer
//!   ├── AndroidDisplay        (display.rs)
//!   ├── gpui_wgpu::CosmicTextSystem
//!   └── jni                   (jni.rs) — event loop + lifecycle
//! ```
//!
//! GPU rendering and text shaping are delegated to the upstream `gpui_wgpu`
//! crate which provides `WgpuRenderer`, `WgpuContext`, and `CosmicTextSystem`.
//!
//! The JNI / ANativeActivity entry-points live in `jni.rs` and are the
//! first Rust code that executes when the Android runtime loads the `.so`.
//!
//! ## Threading model
//!
//! Android's `ALooper` is used as the run-loop.  A dedicated "main" thread
//! created by `ANativeActivity_onCreate` drives the foreground executor, while
//! a Rust `ThreadPool` backs the background executor — matching the two-queue
//! model used on Linux.
//!
//! ## Integration with GPUI
//!
//! This module depends on the `gpui` crate from the Zed repository for all
//! core types: `Platform`, `PlatformWindow`, `PlatformDisplay`, `Pixels`,
//! `DevicePixels`, `Size`, `Point`, `Bounds`, event types, text system traits,
//! etc.  It also depends on `gpui_wgpu` for the wgpu-based renderer and
//! cosmic-text system.
//!
//! This module is only compiled when `target_os = "android"`.

// ── geometry types ───────────────────────────────────────────────────────────
//
// The real `gpui::Pixels` has a `pub(crate)` inner field that is inaccessible
// outside the `gpui` crate, so platform code cannot use `.0` on it.
// We define local geometry stubs here with fully public inner fields.
// Sub-modules import them via `use super::*`.

/// A logical pixel value (CSS px).  Public inner field unlike `gpui::Pixels`.
#[derive(Copy, Clone, Debug, Default, PartialEq, PartialOrd)]
pub struct Pixels(pub f32);

impl Pixels {
    pub const ZERO: Self = Self(0.0);
}

impl From<f32> for Pixels {
    fn from(v: f32) -> Self {
        Self(v)
    }
}

impl From<Pixels> for f32 {
    fn from(p: Pixels) -> Self {
        p.0
    }
}

impl std::ops::Mul<f32> for Pixels {
    type Output = Self;
    fn mul(self, rhs: f32) -> Self {
        Self(self.0 * rhs)
    }
}

impl std::ops::Div<f32> for Pixels {
    type Output = Self;
    fn div(self, rhs: f32) -> Self {
        Self(self.0 / rhs)
    }
}

impl std::ops::Add for Pixels {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0)
    }
}

impl std::ops::Sub for Pixels {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self(self.0 - rhs.0)
    }
}

impl std::ops::Neg for Pixels {
    type Output = Self;
    fn neg(self) -> Self {
        Self(-self.0)
    }
}

/// Physical / device pixels.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DevicePixels(pub i32);

impl From<i32> for DevicePixels {
    fn from(v: i32) -> Self {
        Self(v)
    }
}

impl From<DevicePixels> for i32 {
    fn from(dp: DevicePixels) -> Self {
        dp.0
    }
}

/// A 2-D size.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Size<T> {
    pub width: T,
    pub height: T,
}

/// A 2-D point.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Point<T> {
    pub x: T,
    pub y: T,
}

/// An axis-aligned rectangle.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Bounds<T> {
    pub origin: Point<T>,
    pub size: Size<T>,
}

/// Convenience constructors (mirror `gpui::point` / `gpui::size`).
pub fn point<T>(x: T, y: T) -> Point<T> {
    Point { x, y }
}

pub fn size<T>(width: T, height: T) -> Size<T> {
    Size { width, height }
}

// ── sub-modules ──────────────────────────────────────────────────────────────

pub mod dispatcher;
pub mod display;
mod document_picker;
pub mod jni;
pub mod keyboard;
pub mod platform;
pub mod platform_view;
mod text_input;
pub mod window;

// ── public re-exports ─────────────────────────────────────────────────────────

pub use dispatcher::AndroidDispatcher;
pub use text_input::take_ime_dismissed;
pub use display::AndroidDisplay;
pub use keyboard::*;
pub use platform::{AndroidPlatform, SharedPlatform};
pub use window::{AndroidPlatformWindow, AndroidWindow, SafeAreaInsets};

// ── platform entry-point (mirrors gpui_linux::current_platform) ───────────────

use std::rc::Rc;

/// Returns the Android platform implementation.
///
/// `headless` is accepted for API parity with the Linux / iOS equivalents.
/// When `true` the platform is constructed without an active `ANativeWindow`,
/// which is useful for off-screen rendering and testing.
///
/// # Panics
///
/// Panics if the NDK context has not been initialised (i.e. if called before
/// `ANativeActivity_onCreate` has run).
pub fn current_platform(headless: bool) -> Rc<dyn gpui::Platform> {
    Rc::new(AndroidPlatform::new(headless))
}

// ── helper: Android-specific surface kind ─────────────────────────────────────

/// Which GPU back-end to use for the wgpu surface.
///
/// On Android the Vulkan back-end is strongly preferred; GL-ES is the
/// fallback for devices that don't expose Vulkan 1.1.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum AndroidBackend {
    #[default]
    Vulkan,
    Gles,
}

impl std::fmt::Display for AndroidBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Vulkan => write!(f, "Vulkan"),
            Self::Gles => write!(f, "OpenGL ES"),
        }
    }
}

// ── helper: key / input event types ──────────────────────────────────────────

/// A minimal key event representation.
///
/// Maps the subset of Android `KeyEvent` fields that GPUI needs.
#[derive(Clone, Debug)]
pub struct AndroidKeyEvent {
    /// Android key-code (`android.view.KeyEvent.KEYCODE_*`).
    pub key_code: i32,
    /// `ACTION_DOWN = 0`, `ACTION_UP = 1`.
    pub action: i32,
    /// Modifier bitmask (`META_SHIFT_ON`, `META_CTRL_ON`, …).
    pub meta_state: i32,
    /// Unicode character produced (0 if none).
    pub unicode_char: u32,
}

/// A single touch point from `AInputEvent`.
#[derive(Clone, Debug)]
pub struct TouchPoint {
    pub id: i32,
    pub x: f32,
    pub y: f32,
    /// `AMOTION_EVENT_ACTION_*` action masked to a single pointer.
    pub action: u32,
}

// ── shared logging helper ─────────────────────────────────────────────────────

/// Initialise `android_logger` so that `log::*` macros route to logcat.
///
/// Safe to call multiple times — subsequent calls are no-ops.
pub fn init_logger() {
    use std::sync::OnceLock;
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        android_logger::init_once(
            android_logger::Config::default()
                .with_max_level(log::LevelFilter::Info)
                .with_tag("gpui-android"),
        );
        log::info!("gpui-android logger initialised");
    });
}
