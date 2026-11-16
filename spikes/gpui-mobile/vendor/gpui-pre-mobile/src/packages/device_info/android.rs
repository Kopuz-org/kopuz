use super::DeviceInfo;

pub fn get_device_info() -> Result<DeviceInfo, String> {
    let model = get_system_property("ro.product.model");
    let manufacturer = get_system_property("ro.product.manufacturer");
    let device_name = get_system_property("ro.product.device");
    let os_version = get_system_property("ro.build.version.release");
    let fingerprint = get_system_property("ro.build.fingerprint");

    let is_physical_device = !fingerprint.contains("generic")
        && !fingerprint.contains("emulator")
        && !fingerprint.contains("sdk");

    Ok(DeviceInfo {
        model,
        manufacturer,
        os_version,
        device_name,
        is_physical_device,
    })
}

/// Read an Android system property via the NDK C API.
fn get_system_property(name: &str) -> String {
    // Property name must be null-terminated
    let mut name_buf = Vec::with_capacity(name.len() + 1);
    name_buf.extend_from_slice(name.as_bytes());
    name_buf.push(0);

    // PROP_VALUE_MAX is 92 bytes
    let mut value_buf = [0u8; 92];

    unsafe {
        let len = __system_property_get(
            name_buf.as_ptr() as *const i8,
            value_buf.as_mut_ptr() as *mut i8,
        );
        if len > 0 {
            String::from_utf8_lossy(&value_buf[..len as usize]).into_owned()
        } else {
            String::new()
        }
    }
}

extern "C" {
    fn __system_property_get(name: *const i8, value: *mut i8) -> i32;
}
