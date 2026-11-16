use super::{BarometerData, SensorAvailability, SensorData};
use objc2::encode::{Encode, Encoding, RefEncode};
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};

pub fn available_sensors() -> SensorAvailability {
    unsafe {
        let manager = get_motion_manager();
        if manager.is_null() {
            return SensorAvailability::default();
        }

        SensorAvailability {
            accelerometer: msg_send![manager, isAccelerometerAvailable],
            gyroscope: msg_send![manager, isGyroAvailable],
            magnetometer: msg_send![manager, isMagnetometerAvailable],
            barometer: {
                // CMAltimeter.isRelativeAltitudeAvailable()
                let available: bool = msg_send![class!(CMAltimeter), isRelativeAltitudeAvailable];
                available
            },
        }
    }
}

pub fn accelerometer() -> Option<SensorData> {
    unsafe {
        let manager = get_motion_manager();
        if manager.is_null() {
            return None;
        }

        // Start updates if not already running
        let active: bool = msg_send![manager, isAccelerometerActive];
        if !active {
            let _: () = msg_send![manager, startAccelerometerUpdates];
            // Give the sensor a moment to produce data
            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        let data: *mut AnyObject = msg_send![manager, accelerometerData];
        if data.is_null() {
            return None;
        }
        // CMAcceleration { x, y, z } in g-force; convert to m/s²
        let accel: CMAcceleration = msg_send![data, acceleration];
        Some(SensorData {
            x: accel.x * 9.80665,
            y: accel.y * 9.80665,
            z: accel.z * 9.80665,
        })
    }
}

pub fn gyroscope() -> Option<SensorData> {
    unsafe {
        let manager = get_motion_manager();
        if manager.is_null() {
            return None;
        }

        let active: bool = msg_send![manager, isGyroActive];
        if !active {
            let _: () = msg_send![manager, startGyroUpdates];
            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        let data: *mut AnyObject = msg_send![manager, gyroData];
        if data.is_null() {
            return None;
        }
        let rate: CMRotationRate = msg_send![data, rotationRate];
        Some(SensorData {
            x: rate.x,
            y: rate.y,
            z: rate.z,
        })
    }
}

pub fn magnetometer() -> Option<SensorData> {
    unsafe {
        let manager = get_motion_manager();
        if manager.is_null() {
            return None;
        }

        let active: bool = msg_send![manager, isMagnetometerActive];
        if !active {
            let _: () = msg_send![manager, startMagnetometerUpdates];
            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        let data: *mut AnyObject = msg_send![manager, magnetometerData];
        if data.is_null() {
            return None;
        }
        let field: CMMagneticField = msg_send![data, magneticField];
        Some(SensorData {
            x: field.x,
            y: field.y,
            z: field.z,
        })
    }
}

pub fn barometer() -> Option<BarometerData> {
    // CMAltimeter provides relative altitude changes, not absolute pressure
    // in a simple polling API. The barometer stream requires a handler block.
    // For a polling API, we'd need to cache the last reading from a running
    // altimeter session.
    None
}

// CoreMotion types

macro_rules! encode_xyz_struct {
    ($name:ident) => {
        unsafe impl Encode for $name {
            const ENCODING: Encoding = Encoding::Struct(
                stringify!($name),
                &[Encoding::Double, Encoding::Double, Encoding::Double],
            );
        }
        unsafe impl RefEncode for $name {
            const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
        }
    };
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct CMAcceleration {
    x: f64,
    y: f64,
    z: f64,
}
encode_xyz_struct!(CMAcceleration);

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct CMRotationRate {
    x: f64,
    y: f64,
    z: f64,
}
encode_xyz_struct!(CMRotationRate);

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct CMMagneticField {
    x: f64,
    y: f64,
    z: f64,
}
encode_xyz_struct!(CMMagneticField);

/// Get or create a shared CMMotionManager instance.
///
/// CMMotionManager should be a singleton per app.
unsafe fn get_motion_manager() -> *mut AnyObject {
    use std::sync::OnceLock;
    static MANAGER: OnceLock<usize> = OnceLock::new();

    let ptr = MANAGER.get_or_init(|| {
        let manager: *mut AnyObject = msg_send![class!(CMMotionManager), alloc];
        let manager: *mut AnyObject = msg_send![manager, init];
        manager as usize
    });
    *ptr as *mut AnyObject
}
