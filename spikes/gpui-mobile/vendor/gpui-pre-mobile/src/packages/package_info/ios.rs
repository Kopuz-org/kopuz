use super::PackageInfo;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use std::ffi::CStr;

pub fn get_package_info() -> Result<PackageInfo, String> {
    unsafe {
        let bundle: *mut AnyObject = msg_send![class!(NSBundle), mainBundle];
        if bundle.is_null() {
            return Err("Failed to get NSBundle.mainBundle".into());
        }

        let info_dict: *mut AnyObject = msg_send![bundle, infoDictionary];
        if info_dict.is_null() {
            return Err("Failed to get infoDictionary".into());
        }

        let app_name = nsdict_string(info_dict, "CFBundleDisplayName")
            .or_else(|| nsdict_string(info_dict, "CFBundleName"))
            .unwrap_or_default();

        let package_name = nsdict_string(info_dict, "CFBundleIdentifier").unwrap_or_default();

        let version = nsdict_string(info_dict, "CFBundleShortVersionString").unwrap_or_default();

        let build_number = nsdict_string(info_dict, "CFBundleVersion").unwrap_or_default();

        Ok(PackageInfo {
            app_name,
            package_name,
            version,
            build_number,
        })
    }
}

unsafe fn nsdict_string(dict: *mut AnyObject, key: &str) -> Option<String> {
    let key_nsstring = nsstring(key);
    let value: *mut AnyObject = msg_send![dict, objectForKey: key_nsstring];
    if value.is_null() {
        return None;
    }
    let utf8: *const i8 = msg_send![value, UTF8String];
    if utf8.is_null() {
        return None;
    }
    Some(CStr::from_ptr(utf8).to_string_lossy().into_owned())
}

use crate::ios::util::nsstring;
