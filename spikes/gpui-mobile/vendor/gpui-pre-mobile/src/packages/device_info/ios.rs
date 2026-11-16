use super::DeviceInfo;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use std::ffi::CStr;

pub fn get_device_info() -> Result<DeviceInfo, String> {
    unsafe {
        // UIDevice.currentDevice
        let device: *mut AnyObject = msg_send![class!(UIDevice), currentDevice];
        if device.is_null() {
            return Err("Failed to get UIDevice.currentDevice".into());
        }

        let device_name = nsstring_to_string(msg_send![device, name]);
        let os_version = nsstring_to_string(msg_send![device, systemVersion]);

        // Get hardware model via uname() syscall
        let model = get_machine_model();

        // Check if running on simulator
        let is_physical_device = !cfg!(target_os = "ios") || !is_simulator();

        Ok(DeviceInfo {
            model,
            manufacturer: "Apple".to_string(),
            os_version,
            device_name,
            is_physical_device,
        })
    }
}

fn get_machine_model() -> String {
    let mut info: libc::utsname = unsafe { std::mem::zeroed() };
    let ret = unsafe { libc::uname(&mut info) };
    if ret == 0 {
        unsafe {
            CStr::from_ptr(info.machine.as_ptr())
                .to_string_lossy()
                .into_owned()
        }
    } else {
        "Unknown".to_string()
    }
}

fn is_simulator() -> bool {
    // On iOS simulators, the architecture is x86_64 or arm64 on Apple Silicon
    // but the TARGET_OS_SIMULATOR preprocessor flag is set. In Rust we check
    // the machine field from uname.
    let model = get_machine_model();
    model == "x86_64" || model == "arm64" || model.contains("simulator")
}

unsafe fn nsstring_to_string(ns: *mut AnyObject) -> String {
    if ns.is_null() {
        return String::new();
    }
    let utf8: *const i8 = msg_send![ns, UTF8String];
    if utf8.is_null() {
        return String::new();
    }
    CStr::from_ptr(utf8).to_string_lossy().into_owned()
}

// libc types needed for uname
mod libc {
    #[repr(C)]
    pub struct utsname {
        pub sysname: [i8; 256],
        pub nodename: [i8; 256],
        pub release: [i8; 256],
        pub version: [i8; 256],
        pub machine: [i8; 256],
    }

    extern "C" {
        pub fn uname(buf: *mut utsname) -> i32;
    }
}
