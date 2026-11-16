//! Maps launcher for opening native maps applications.
//!
//! Provides a cross-platform API to launch maps apps backed by:
//! - Android: Geo intent (opens Google Maps or default maps app)
//! - iOS: Apple Maps URL scheme
//!
//! Feature-gated behind `maps_launcher`.

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "ios")]
mod ios;

/// Launch maps app showing a specific coordinate.
///
/// If `label` is provided, it will be shown as a marker title at the location.
/// Returns `Ok(true)` if the maps app was opened successfully.
pub fn open_coordinates(
    latitude: f64,
    longitude: f64,
    label: Option<&str>,
) -> Result<bool, String> {
    #[cfg(target_os = "ios")]
    {
        ios::open_coordinates(latitude, longitude, label)
    }
    #[cfg(target_os = "android")]
    {
        android::open_coordinates(latitude, longitude, label)
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        let _ = (latitude, longitude, label);
        Err("maps_launcher is only available on iOS and Android".into())
    }
}

/// Launch maps app with a search query (e.g. address or place name).
///
/// Returns `Ok(true)` if the maps app was opened successfully.
pub fn open_query(query: &str) -> Result<bool, String> {
    #[cfg(target_os = "ios")]
    {
        ios::open_query(query)
    }
    #[cfg(target_os = "android")]
    {
        android::open_query(query)
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        let _ = query;
        Err("maps_launcher is only available on iOS and Android".into())
    }
}

/// Launch maps app showing directions from current location to a destination.
///
/// If `dest_label` is provided, it will be used as the destination name.
/// Returns `Ok(true)` if the maps app was opened successfully.
pub fn open_directions(
    dest_latitude: f64,
    dest_longitude: f64,
    dest_label: Option<&str>,
) -> Result<bool, String> {
    #[cfg(target_os = "ios")]
    {
        ios::open_directions(dest_latitude, dest_longitude, dest_label)
    }
    #[cfg(target_os = "android")]
    {
        android::open_directions(dest_latitude, dest_longitude, dest_label)
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        let _ = (dest_latitude, dest_longitude, dest_label);
        Err("maps_launcher is only available on iOS and Android".into())
    }
}

/// Check if a maps app is available on the device.
///
/// On iOS this always returns `Ok(true)` since Apple Maps is built in.
pub fn is_available() -> Result<bool, String> {
    #[cfg(target_os = "ios")]
    {
        ios::is_available()
    }
    #[cfg(target_os = "android")]
    {
        android::is_available()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        Err("maps_launcher is only available on iOS and Android".into())
    }
}
