//! Location services for GPS/network position.
//!
//! Provides a cross-platform location API backed by:
//! - Android: FusedLocationProviderClient / LocationManager via JNI
//! - iOS: CLLocationManager via Objective-C
//!
//! Feature-gated behind `location`.

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "ios")]
mod ios;

/// GPS position data.
#[derive(Debug, Clone, Copy)]
pub struct Position {
    pub latitude: f64,
    pub longitude: f64,
    pub altitude: f64,
    pub accuracy: f64, // meters
    pub speed: f64,    // m/s
    pub speed_accuracy: f64,
    pub heading: f64, // degrees
    pub heading_accuracy: f64,
    pub timestamp: u64, // unix millis
}

/// Location accuracy setting.
#[derive(Debug, Clone, Copy, Default)]
pub enum LocationAccuracy {
    Lowest,
    Low,
    Medium,
    #[default]
    High,
    Best,
    BestForNavigation,
}

/// Location settings for requests.
#[derive(Debug, Clone)]
pub struct LocationSettings {
    pub accuracy: LocationAccuracy,
    pub distance_filter: f64, // meters, minimum displacement to report
}

impl Default for LocationSettings {
    fn default() -> Self {
        Self {
            accuracy: LocationAccuracy::High,
            distance_filter: 0.0,
        }
    }
}

/// Whether location services are enabled on the device.
pub fn is_location_service_enabled() -> Result<bool, String> {
    #[cfg(target_os = "ios")]
    {
        ios::is_location_service_enabled()
    }
    #[cfg(target_os = "android")]
    {
        android::is_location_service_enabled()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        Ok(false)
    }
}

/// Get the current position (one-shot).
#[allow(unused_variables)]
pub fn get_current_position(settings: &LocationSettings) -> Result<Position, String> {
    #[cfg(target_os = "ios")]
    {
        ios::get_current_position(settings)
    }
    #[cfg(target_os = "android")]
    {
        android::get_current_position(settings)
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        Err("Location not supported on this platform".into())
    }
}

/// Get the last known position (may be stale, faster than current).
pub fn get_last_known_position() -> Result<Option<Position>, String> {
    #[cfg(target_os = "ios")]
    {
        ios::get_last_known_position()
    }
    #[cfg(target_os = "android")]
    {
        android::get_last_known_position()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        Ok(None)
    }
}

/// Calculate distance between two positions in meters (Haversine formula).
pub fn distance_between(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let r = 6_371_000.0; // Earth radius in meters
    let d_lat = (lat2 - lat1).to_radians();
    let d_lon = (lon2 - lon1).to_radians();
    let a = (d_lat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (d_lon / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().asin();
    r * c
}

/// Calculate bearing from point 1 to point 2 in degrees (0-360).
pub fn bearing_between(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let lat1_r = lat1.to_radians();
    let lat2_r = lat2.to_radians();
    let d_lon = (lon2 - lon1).to_radians();

    let x = d_lon.sin() * lat2_r.cos();
    let y = lat1_r.cos() * lat2_r.sin() - lat1_r.sin() * lat2_r.cos() * d_lon.cos();

    let bearing = x.atan2(y).to_degrees();
    (bearing + 360.0) % 360.0
}
