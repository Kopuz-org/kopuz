//! Battery level, charging state, and battery saver mode.

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "ios")]
mod ios;

/// Battery charging state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatteryState {
    /// Device is charging.
    Charging,
    /// Device is running on battery (discharging).
    Discharging,
    /// Battery is full.
    Full,
    /// Battery state cannot be determined.
    Unknown,
}

/// Battery information snapshot.
#[derive(Debug, Clone)]
pub struct BatteryInfo {
    /// Battery level as a percentage (0–100). `-1` if unavailable.
    pub level: i32,
    /// Current charging state.
    pub state: BatteryState,
    /// Whether the device is in battery saver / low-power mode.
    pub is_battery_save_mode: bool,
}

/// Get the current battery level as a percentage (0–100).
///
/// Returns `-1` if the battery level cannot be determined.
pub fn battery_level() -> i32 {
    #[cfg(target_os = "ios")]
    {
        ios::battery_level()
    }
    #[cfg(target_os = "android")]
    {
        android::battery_level()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        -1
    }
}

/// Get the current battery charging state.
pub fn battery_state() -> BatteryState {
    #[cfg(target_os = "ios")]
    {
        ios::battery_state()
    }
    #[cfg(target_os = "android")]
    {
        android::battery_state()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        BatteryState::Unknown
    }
}

/// Check whether the device is in battery saver / low-power mode.
pub fn is_battery_save_mode() -> bool {
    #[cfg(target_os = "ios")]
    {
        ios::is_battery_save_mode()
    }
    #[cfg(target_os = "android")]
    {
        android::is_battery_save_mode()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        false
    }
}

/// Get a full battery info snapshot.
pub fn battery_info() -> BatteryInfo {
    BatteryInfo {
        level: battery_level(),
        state: battery_state(),
        is_battery_save_mode: is_battery_save_mode(),
    }
}
