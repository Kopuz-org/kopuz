//! Network connectivity status.

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "ios")]
mod ios;

/// Current network connectivity status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectivityStatus {
    /// Connected via WiFi.
    Wifi,
    /// Connected via cellular (mobile data).
    Cellular,
    /// No network connection.
    None,
}

/// Check the current network connectivity status.
pub fn check_connectivity() -> ConnectivityStatus {
    #[cfg(target_os = "ios")]
    {
        ios::check_connectivity()
    }
    #[cfg(target_os = "android")]
    {
        android::check_connectivity()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        ConnectivityStatus::None
    }
}
