use objc2::msg_send;
use objc2::runtime::AnyObject;
use std::ffi::CStr;
use std::path::PathBuf;

pub fn temporary_directory() -> Result<PathBuf, String> {
    unsafe {
        extern "C" {
            fn NSTemporaryDirectory() -> *mut AnyObject;
        }
        let path = NSTemporaryDirectory();
        nsstring_to_pathbuf(path)
    }
}

pub fn documents_directory() -> Result<PathBuf, String> {
    search_path_directory(9) // NSDocumentDirectory
}

pub fn cache_directory() -> Result<PathBuf, String> {
    search_path_directory(13) // NSCachesDirectory
}

pub fn support_directory() -> Result<PathBuf, String> {
    search_path_directory(14) // NSApplicationSupportDirectory
}

/// Calls NSSearchPathForDirectoriesInDomains(directory, NSUserDomainMask, YES)
fn search_path_directory(directory: u64) -> Result<PathBuf, String> {
    unsafe {
        extern "C" {
            fn NSSearchPathForDirectoriesInDomains(
                directory: u64,
                domain_mask: u64,
                expand_tilde: bool,
            ) -> *mut AnyObject; // NSArray<NSString*>
        }

        let array = NSSearchPathForDirectoriesInDomains(
            directory, 1, // NSUserDomainMask
            true,
        );
        if array.is_null() {
            return Err("NSSearchPathForDirectoriesInDomains returned nil".into());
        }

        let count: u64 = msg_send![array, count];
        if count == 0 {
            return Err("NSSearchPathForDirectoriesInDomains returned empty array".into());
        }

        let first: *mut AnyObject = msg_send![array, objectAtIndex: 0u64];
        nsstring_to_pathbuf(first)
    }
}

unsafe fn nsstring_to_pathbuf(ns: *mut AnyObject) -> Result<PathBuf, String> {
    if ns.is_null() {
        return Err("NSString is nil".into());
    }
    let utf8: *const i8 = msg_send![ns, UTF8String];
    if utf8.is_null() {
        return Err("UTF8String returned null".into());
    }
    let s = CStr::from_ptr(utf8).to_string_lossy().into_owned();
    Ok(PathBuf::from(s))
}
