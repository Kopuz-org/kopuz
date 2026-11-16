//! Device hardware and OS information.

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "ios")]
mod ios;

/// Device hardware and operating system metadata.
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    /// Device model identifier (e.g. "iPhone14,5", "Pixel 6").
    pub model: String,
    /// Device manufacturer (e.g. "Apple", "Samsung").
    pub manufacturer: String,
    /// OS version string (e.g. "17.0", "14").
    pub os_version: String,
    /// User-assigned device name (e.g. "John's iPhone").
    pub device_name: String,
    /// Whether the app is running on a physical device (vs. simulator/emulator).
    pub is_physical_device: bool,
}

/// Retrieve information about the current device.
pub fn get_device_info() -> Result<DeviceInfo, String> {
    #[cfg(target_os = "ios")]
    {
        ios::get_device_info()
    }
    #[cfg(target_os = "android")]
    {
        android::get_device_info()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        Err("device_info is only available on iOS and Android".into())
    }
}
