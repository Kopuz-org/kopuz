//! Network interface information (WiFi name, BSSID, IP address).

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "ios")]
mod ios;

/// Information about the current network connection.
#[derive(Debug, Clone, Default)]
pub struct NetworkInfo {
    /// WiFi network name (SSID), if connected to WiFi.
    pub wifi_name: Option<String>,
    /// WiFi access point BSSID, if connected to WiFi.
    pub wifi_bssid: Option<String>,
    /// Device's IP address on the current network.
    pub wifi_ip: Option<String>,
}

/// Retrieve information about the current network connection.
///
/// Note: On iOS 12+ reading the WiFi SSID requires the
/// "Access WiFi Information" entitlement and location permission.
pub fn get_network_info() -> Result<NetworkInfo, String> {
    #[cfg(target_os = "ios")]
    {
        ios::get_network_info()
    }
    #[cfg(target_os = "android")]
    {
        android::get_network_info()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        Err("network_info is only available on iOS and Android".into())
    }
}
