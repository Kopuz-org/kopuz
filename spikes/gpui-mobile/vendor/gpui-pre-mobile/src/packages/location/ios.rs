use super::{LocationAccuracy, LocationSettings, Position};
use objc2::encode::{Encode, Encoding, RefEncode};
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use std::sync::OnceLock;

/// CLLocation coordinate struct matching CoreLocation's CLLocationCoordinate2D.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct CLLocationCoordinate2D {
    latitude: f64,
    longitude: f64,
}

unsafe impl Encode for CLLocationCoordinate2D {
    const ENCODING: Encoding = Encoding::Struct(
        "CLLocationCoordinate2D",
        &[Encoding::Double, Encoding::Double],
    );
}

unsafe impl RefEncode for CLLocationCoordinate2D {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
}

pub fn is_location_service_enabled() -> Result<bool, String> {
    unsafe {
        let enabled: bool = msg_send![class!(CLLocationManager), locationServicesEnabled];
        Ok(enabled)
    }
}

pub fn get_current_position(settings: &LocationSettings) -> Result<Position, String> {
    unsafe {
        let manager = get_location_manager();
        if manager.is_null() {
            return Err("Failed to create CLLocationManager".into());
        }

        // Set desired accuracy
        let accuracy = cl_accuracy(settings.accuracy);
        let _: () = msg_send![manager, setDesiredAccuracy: accuracy];

        // Set distance filter
        if settings.distance_filter > 0.0 {
            let _: () = msg_send![manager, setDistanceFilter: settings.distance_filter];
        }

        // Request a single location update
        let _: () = msg_send![manager, requestLocation];

        // Give CLLocationManager time to acquire a fix.
        // In a production app this would use a delegate callback; here we poll.
        let mut location: *mut AnyObject = std::ptr::null_mut();
        for _ in 0..60 {
            std::thread::sleep(std::time::Duration::from_millis(500));
            location = msg_send![manager, location];
            if !location.is_null() {
                break;
            }
        }

        if location.is_null() {
            return Err("Timed out waiting for location".into());
        }

        Ok(parse_cl_location(location))
    }
}

pub fn get_last_known_position() -> Result<Option<Position>, String> {
    unsafe {
        let manager = get_location_manager();
        if manager.is_null() {
            return Err("Failed to create CLLocationManager".into());
        }

        let location: *mut AnyObject = msg_send![manager, location];
        if location.is_null() {
            return Ok(None);
        }

        Ok(Some(parse_cl_location(location)))
    }
}

unsafe fn parse_cl_location(location: *mut AnyObject) -> Position {
    let coord: CLLocationCoordinate2D = msg_send![location, coordinate];
    let altitude: f64 = msg_send![location, altitude];
    let horizontal_accuracy: f64 = msg_send![location, horizontalAccuracy];
    let speed: f64 = msg_send![location, speed];
    let speed_accuracy: f64 = msg_send![location, speedAccuracy];
    let course: f64 = msg_send![location, course];
    let course_accuracy: f64 = msg_send![location, courseAccuracy];

    // Get timestamp as unix millis from NSDate
    let timestamp_obj: *mut AnyObject = msg_send![location, timestamp];
    let time_interval: f64 = msg_send![timestamp_obj, timeIntervalSince1970];
    let timestamp_millis = (time_interval * 1000.0) as u64;

    Position {
        latitude: coord.latitude,
        longitude: coord.longitude,
        altitude,
        accuracy: horizontal_accuracy,
        speed: if speed < 0.0 { 0.0 } else { speed },
        speed_accuracy: if speed_accuracy < 0.0 {
            0.0
        } else {
            speed_accuracy
        },
        heading: if course < 0.0 { 0.0 } else { course },
        heading_accuracy: if course_accuracy < 0.0 {
            0.0
        } else {
            course_accuracy
        },
        timestamp: timestamp_millis,
    }
}

/// Map LocationAccuracy to CLLocationAccuracy constant values.
fn cl_accuracy(accuracy: LocationAccuracy) -> f64 {
    match accuracy {
        // kCLLocationAccuracyThreeKilometers
        LocationAccuracy::Lowest => 3000.0,
        // kCLLocationAccuracyKilometer
        LocationAccuracy::Low => 1000.0,
        // kCLLocationAccuracyHundredMeters
        LocationAccuracy::Medium => 100.0,
        // kCLLocationAccuracyNearestTenMeters
        LocationAccuracy::High => 10.0,
        // kCLLocationAccuracyBest
        LocationAccuracy::Best => -1.0,
        // kCLLocationAccuracyBestForNavigation
        LocationAccuracy::BestForNavigation => -2.0,
    }
}

/// Get or create a shared CLLocationManager instance.
///
/// CLLocationManager is typically used as a singleton per app.
unsafe fn get_location_manager() -> *mut AnyObject {
    static MANAGER: OnceLock<usize> = OnceLock::new();

    let ptr = MANAGER.get_or_init(|| {
        let manager: *mut AnyObject = msg_send![class!(CLLocationManager), alloc];
        let manager: *mut AnyObject = msg_send![manager, init];
        manager as usize
    });
    *ptr as *mut AnyObject
}
