//! Runtime permission handling for iOS and Android.
//!
//! Provides a cross-platform API for checking, requesting, and managing
//! app permissions backed by:
//! - iOS: `AVFoundation`, `CoreLocation`, `Photos`, `Contacts`, `EventKit`,
//!   `UserNotifications`, `CoreBluetooth`, `CoreMotion`, `Speech` frameworks
//! - Android: `ActivityCompat.requestPermissions` / `ContextCompat.checkSelfPermission` via JNI
//!
//! Inspired by [permission_handler](https://pub.dev/packages/permission_handler).
//!
//! Feature-gated behind `permission_handler`.

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "ios")]
mod ios;

/// A permission that can be requested from the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Permission {
    /// Camera access.
    Camera,
    /// Microphone access.
    Microphone,
    /// Location access while the app is in use.
    LocationWhenInUse,
    /// Location access at all times (background).
    LocationAlways,
    /// Read contacts.
    Contacts,
    /// Read/write calendar events.
    Calendar,
    /// Read/write reminders (iOS only).
    Reminders,
    /// Access the photo library.
    Photos,
    /// Access media library / music (iOS only).
    MediaLibrary,
    /// Access device sensors (accelerometer, gyroscope).
    Sensors,
    /// Bluetooth access.
    Bluetooth,
    /// Post notifications.
    Notification,
    /// Read external storage (Android only, pre-API 33).
    Storage,
    /// Speech recognition.
    Speech,
    /// App tracking transparency (iOS 14+).
    AppTrackingTransparency,
    /// Display system alert window / overlay (Android only).
    SystemAlertWindow,
    /// Install packages from unknown sources (Android only).
    InstallPackages,
    /// Access notification policy / Do Not Disturb (Android only).
    AccessNotificationPolicy,
    /// Read phone state (Android only).
    Phone,
    /// Read SMS (Android only).
    Sms,
    /// Access videos (Android 13+ / iOS).
    Videos,
    /// Access audio files (Android 13+).
    Audio,
}

/// The status of a permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionStatus {
    /// The user has granted the permission.
    Granted,
    /// The user has denied the permission (can still be requested again).
    Denied,
    /// The OS restricts access (e.g. parental controls on iOS).
    Restricted,
    /// The user has permanently denied the permission.
    /// On Android this means "Don't ask again" was checked.
    /// The user must go to Settings to grant it.
    PermanentlyDenied,
    /// The permission is granted with limitations (e.g. iOS limited photo access).
    Limited,
    /// The permission is provisionally granted (e.g. iOS provisional notifications).
    Provisional,
}

impl PermissionStatus {
    pub fn is_granted(self) -> bool {
        matches!(self, Self::Granted | Self::Limited | Self::Provisional)
    }
    pub fn is_denied(self) -> bool {
        matches!(self, Self::Denied)
    }
    pub fn is_permanently_denied(self) -> bool {
        matches!(self, Self::PermanentlyDenied)
    }
    pub fn is_restricted(self) -> bool {
        matches!(self, Self::Restricted)
    }
    pub fn is_limited(self) -> bool {
        matches!(self, Self::Limited)
    }
}

/// The status of a platform service associated with a permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceStatus {
    /// The service is enabled (e.g. Location Services is on).
    Enabled,
    /// The service is disabled.
    Disabled,
    /// The service status is not applicable for this permission.
    NotApplicable,
}

/// Check the current status of a permission without prompting the user.
pub fn check_permission(permission: Permission) -> Result<PermissionStatus, String> {
    #[cfg(target_os = "ios")]
    {
        ios::check_permission(permission)
    }
    #[cfg(target_os = "android")]
    {
        android::check_permission(permission)
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        let _ = permission;
        Err("permission_handler is only available on iOS and Android".into())
    }
}

/// Request a permission from the user.
///
/// If the permission has already been granted, returns `Granted` immediately.
/// If the permission was permanently denied, returns `PermanentlyDenied`.
/// Otherwise, shows the system permission dialog and returns the user's choice.
pub fn request_permission(permission: Permission) -> Result<PermissionStatus, String> {
    #[cfg(target_os = "ios")]
    {
        ios::request_permission(permission)
    }
    #[cfg(target_os = "android")]
    {
        android::request_permission(permission)
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        let _ = permission;
        Err("permission_handler is only available on iOS and Android".into())
    }
}

/// Request multiple permissions at once.
///
/// Returns a map of permission → status for each requested permission.
pub fn request_permissions(
    permissions: &[Permission],
) -> Result<Vec<(Permission, PermissionStatus)>, String> {
    #[cfg(target_os = "ios")]
    {
        ios::request_permissions(permissions)
    }
    #[cfg(target_os = "android")]
    {
        android::request_permissions(permissions)
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        let _ = permissions;
        Err("permission_handler is only available on iOS and Android".into())
    }
}

/// Check the status of a platform service associated with a permission.
///
/// For example, `Permission::LocationWhenInUse` checks if Location Services is enabled.
pub fn service_status(permission: Permission) -> Result<ServiceStatus, String> {
    #[cfg(target_os = "ios")]
    {
        ios::service_status(permission)
    }
    #[cfg(target_os = "android")]
    {
        android::service_status(permission)
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        let _ = permission;
        Err("permission_handler is only available on iOS and Android".into())
    }
}

/// Open the app's settings page so the user can manually change permissions.
///
/// This is useful when a permission is permanently denied.
pub fn open_app_settings() -> Result<bool, String> {
    #[cfg(target_os = "ios")]
    {
        ios::open_app_settings()
    }
    #[cfg(target_os = "android")]
    {
        android::open_app_settings()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        Err("permission_handler is only available on iOS and Android".into())
    }
}

/// Check whether the app should show a rationale for requesting a permission.
///
/// On Android, returns `true` if the user has previously denied the permission
/// but has not checked "Don't ask again". On iOS, always returns `false`.
pub fn should_show_request_rationale(permission: Permission) -> Result<bool, String> {
    #[cfg(target_os = "ios")]
    {
        let _ = permission;
        Ok(false)
    }
    #[cfg(target_os = "android")]
    {
        android::should_show_request_rationale(permission)
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        let _ = permission;
        Err("permission_handler is only available on iOS and Android".into())
    }
}
