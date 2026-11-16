use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use std::ffi::CString;

#[link(name = "StoreKit", kind = "framework")]
extern "C" {}

pub fn is_available() -> Result<bool, String> {
    // SKStoreReviewController is available on all supported iOS versions (10.3+).
    Ok(true)
}

pub fn request_review() -> Result<(), String> {
    unsafe {
        let cls = class!(SKStoreReviewController);
        let _: () = msg_send![cls, requestReview];
    }
    Ok(())
}

pub fn open_store_listing(app_id: &str) -> Result<(), String> {
    unsafe {
        let url_str = format!("https://apps.apple.com/app/id{}", app_id);
        let c_url_str = CString::new(url_str).map_err(|e| e.to_string())?;
        let ns_url_str: *mut AnyObject = msg_send![class!(NSString), alloc];
        let ns_url_str: *mut AnyObject =
            msg_send![ns_url_str, initWithUTF8String: c_url_str.as_ptr()];
        if ns_url_str.is_null() {
            return Err("Failed to create NSString for URL".into());
        }
        let url: *mut AnyObject = msg_send![class!(NSURL), URLWithString: ns_url_str];
        if url.is_null() {
            return Err("Failed to create NSURL".into());
        }
        let app: *mut AnyObject = msg_send![class!(UIApplication), sharedApplication];
        if app.is_null() {
            return Err("Failed to get UIApplication".into());
        }
        let _: () = msg_send![app, openURL: url];
    }
    Ok(())
}
