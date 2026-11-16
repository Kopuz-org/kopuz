//! Device sensor access — accelerometer, gyroscope, magnetometer, barometer.

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "ios")]
mod ios;

/// A 3-axis sensor reading (x, y, z) in SI units.
///
/// - Accelerometer: m/s²
/// - Gyroscope: rad/s
/// - Magnetometer: µT (microtesla)
#[derive(Debug, Clone, Copy, Default)]
pub struct SensorData {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// Atmospheric pressure reading.
#[derive(Debug, Clone, Copy, Default)]
pub struct BarometerData {
    /// Pressure in hectopascals (hPa).
    pub pressure: f64,
}

/// Which sensor types are available on this device.
#[derive(Debug, Clone, Copy, Default)]
pub struct SensorAvailability {
    pub accelerometer: bool,
    pub gyroscope: bool,
    pub magnetometer: bool,
    pub barometer: bool,
}

/// Check which sensors are available on this device.
pub fn available_sensors() -> SensorAvailability {
    #[cfg(target_os = "ios")]
    {
        ios::available_sensors()
    }
    #[cfg(target_os = "android")]
    {
        android::available_sensors()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        SensorAvailability::default()
    }
}

/// Get the latest accelerometer reading (includes gravity).
pub fn accelerometer() -> Option<SensorData> {
    #[cfg(target_os = "ios")]
    {
        ios::accelerometer()
    }
    #[cfg(target_os = "android")]
    {
        android::accelerometer()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        None
    }
}

/// Get the latest gyroscope reading.
pub fn gyroscope() -> Option<SensorData> {
    #[cfg(target_os = "ios")]
    {
        ios::gyroscope()
    }
    #[cfg(target_os = "android")]
    {
        android::gyroscope()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        None
    }
}

/// Get the latest magnetometer reading.
pub fn magnetometer() -> Option<SensorData> {
    #[cfg(target_os = "ios")]
    {
        ios::magnetometer()
    }
    #[cfg(target_os = "android")]
    {
        android::magnetometer()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        None
    }
}

/// Get the latest barometer (atmospheric pressure) reading.
pub fn barometer() -> Option<BarometerData> {
    #[cfg(target_os = "ios")]
    {
        ios::barometer()
    }
    #[cfg(target_os = "android")]
    {
        android::barometer()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        None
    }
}
