//! Runtime platform detection.
//!
//! Provides a `TargetPlatform` enum and `target_platform()` function so
//! application code can query the current platform at runtime without
//! `#[cfg]` attributes everywhere.

/// The platform the application is currently running on.
///
/// Similar to Flutter's `TargetPlatform`, this enum allows runtime branching
/// based on the host OS. For compile-time conditional compilation, continue
/// using `#[cfg(target_os = "...")]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TargetPlatform {
    /// Android (NDK / JNI).
    Android,
    /// iOS (UIKit / Metal).
    IOS,
    /// macOS (AppKit / Metal).
    MacOS,
    /// Linux (X11 / Wayland).
    Linux,
    /// Windows (Win32 / DirectX).
    Windows,
    /// Web (wasm32 / WebGPU).
    Web,
}

impl TargetPlatform {
    /// Returns `true` if this is a mobile platform (Android or iOS).
    pub fn is_mobile(self) -> bool {
        matches!(self, Self::Android | Self::IOS)
    }

    /// Returns `true` if this is a desktop platform (macOS, Linux, or Windows).
    pub fn is_desktop(self) -> bool {
        matches!(self, Self::MacOS | Self::Linux | Self::Windows)
    }

    /// Returns `true` if this is Android.
    pub fn is_android(self) -> bool {
        self == Self::Android
    }

    /// Returns `true` if this is iOS.
    pub fn is_ios(self) -> bool {
        self == Self::IOS
    }

    /// Returns `true` if this is macOS.
    pub fn is_macos(self) -> bool {
        self == Self::MacOS
    }

    /// Returns `true` if this is Linux.
    pub fn is_linux(self) -> bool {
        self == Self::Linux
    }

    /// Returns `true` if this is Windows.
    pub fn is_windows(self) -> bool {
        self == Self::Windows
    }

    /// Returns `true` if this is a web (WASM) target.
    pub fn is_web(self) -> bool {
        self == Self::Web
    }

    /// Returns `true` if this is an Apple platform (iOS or macOS).
    pub fn is_apple(self) -> bool {
        matches!(self, Self::IOS | Self::MacOS)
    }
}

impl std::fmt::Display for TargetPlatform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Android => write!(f, "Android"),
            Self::IOS => write!(f, "iOS"),
            Self::MacOS => write!(f, "macOS"),
            Self::Linux => write!(f, "Linux"),
            Self::Windows => write!(f, "Windows"),
            Self::Web => write!(f, "Web"),
        }
    }
}

/// Returns the `TargetPlatform` for the current compilation target.
///
/// This is resolved at compile time via `cfg` — there is no runtime overhead.
///
/// # Examples
///
/// ```rust
/// use gpui_mobile::target_platform;
///
/// let platform = target_platform();
/// if platform.is_mobile() {
///     // Mobile-specific UI adjustments
/// }
/// ```
pub fn target_platform() -> TargetPlatform {
    #[cfg(target_os = "android")]
    {
        TargetPlatform::Android
    }
    #[cfg(target_os = "ios")]
    {
        TargetPlatform::IOS
    }
    #[cfg(target_os = "macos")]
    {
        return TargetPlatform::MacOS;
    }
    #[cfg(target_os = "linux")]
    {
        return TargetPlatform::Linux;
    }
    #[cfg(target_os = "windows")]
    {
        return TargetPlatform::Windows;
    }
    #[cfg(target_arch = "wasm32")]
    {
        return TargetPlatform::Web;
    }

    #[cfg(not(any(
        target_os = "android",
        target_os = "ios",
        target_os = "macos",
        target_os = "linux",
        target_os = "windows",
        target_arch = "wasm32",
    )))]
    {
        panic!(
            "gpui_mobile::target_platform() does not support this target. \
             Add a new TargetPlatform variant for your platform."
        )
    }
}

/// The default platform for the current compilation target.
///
/// This is a `const` value resolved at compile time that can be used in
/// `const` contexts where the `target_platform()` function cannot.
pub const DEFAULT_PLATFORM: TargetPlatform = {
    #[cfg(target_os = "android")]
    {
        TargetPlatform::Android
    }
    #[cfg(target_os = "ios")]
    {
        TargetPlatform::IOS
    }
    #[cfg(target_os = "macos")]
    {
        TargetPlatform::MacOS
    }
    #[cfg(target_os = "linux")]
    {
        TargetPlatform::Linux
    }
    #[cfg(target_os = "windows")]
    {
        TargetPlatform::Windows
    }
    #[cfg(target_arch = "wasm32")]
    {
        TargetPlatform::Web
    }
    // Fallback for unknown targets — default to Linux so that `cargo doc`
    // and CI builds on uncommon hosts (e.g. FreeBSD) do not fail.
    #[cfg(not(any(
        target_os = "android",
        target_os = "ios",
        target_os = "macos",
        target_os = "linux",
        target_os = "windows",
        target_arch = "wasm32",
    )))]
    {
        TargetPlatform::Linux
    }
};
