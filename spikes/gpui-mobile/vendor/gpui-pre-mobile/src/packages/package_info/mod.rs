//! Application package/bundle information.

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "ios")]
mod ios;

/// Application package metadata.
#[derive(Debug, Clone)]
pub struct PackageInfo {
    /// Human-readable application name.
    pub app_name: String,
    /// Bundle identifier (iOS) or application ID (Android).
    pub package_name: String,
    /// User-facing version string (e.g. "1.2.3").
    pub version: String,
    /// Build number (e.g. "42").
    pub build_number: String,
}

/// Retrieve the current application's package info.
pub fn get_package_info() -> Result<PackageInfo, String> {
    #[cfg(target_os = "ios")]
    {
        ios::get_package_info()
    }
    #[cfg(target_os = "android")]
    {
        android::get_package_info()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        Err("package_info is only available on iOS and Android".into())
    }
}
