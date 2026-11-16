use super::BatteryState;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};

pub fn battery_level() -> i32 {
    unsafe {
        let device: *mut AnyObject = msg_send![class!(UIDevice), currentDevice];
        let _: () = msg_send![device, setBatteryMonitoringEnabled: true];
        let level: f32 = msg_send![device, batteryLevel];
        if level < 0.0 {
            -1
        } else {
            (level * 100.0).round() as i32
        }
    }
}

pub fn battery_state() -> BatteryState {
    unsafe {
        let device: *mut AnyObject = msg_send![class!(UIDevice), currentDevice];
        let _: () = msg_send![device, setBatteryMonitoringEnabled: true];
        let state: i64 = msg_send![device, batteryState];
        // UIDeviceBatteryState: 0=unknown, 1=unplugged, 2=charging, 3=full
        match state {
            1 => BatteryState::Discharging,
            2 => BatteryState::Charging,
            3 => BatteryState::Full,
            _ => BatteryState::Unknown,
        }
    }
}

pub fn is_battery_save_mode() -> bool {
    unsafe {
        let process_info: *mut AnyObject = msg_send![class!(NSProcessInfo), processInfo];
        let low_power: bool = msg_send![process_info, isLowPowerModeEnabled];
        low_power
    }
}
