use objc2::runtime::AnyObject;
use objc2::{class, msg_send};

pub fn launch_url(url: &str) -> Result<bool, String> {
    unsafe {
        let nsurl = nsurl_from_str(url)?;
        let app: *mut AnyObject = msg_send![class!(UIApplication), sharedApplication];
        if app.is_null() {
            return Err("Failed to get UIApplication.sharedApplication".into());
        }
        let result: bool = msg_send![app, openURL: nsurl];
        Ok(result)
    }
}

pub fn can_launch_url(url: &str) -> Result<bool, String> {
    unsafe {
        let nsurl = nsurl_from_str(url)?;
        let app: *mut AnyObject = msg_send![class!(UIApplication), sharedApplication];
        if app.is_null() {
            return Err("Failed to get UIApplication.sharedApplication".into());
        }
        let result: bool = msg_send![app, canOpenURL: nsurl];
        Ok(result)
    }
}

unsafe fn nsurl_from_str(url: &str) -> Result<*mut AnyObject, String> {
    let ns_string: *mut AnyObject = msg_send![class!(NSString), alloc];
    let ns_string: *mut AnyObject = msg_send![ns_string, initWithBytes: url.as_ptr().cast::<std::ffi::c_void>(),
                                                       length: url.len(),
                                                       encoding: 4u64];
    if ns_string.is_null() {
        return Err("Failed to create NSString from URL".into());
    }
    let nsurl: *mut AnyObject = msg_send![class!(NSURL), URLWithString: ns_string];
    // ns_string was alloc+init (retained); release now that URLWithString: has consumed it
    let _: () = msg_send![ns_string, release];
    if nsurl.is_null() {
        return Err(format!("Failed to parse URL: {url}"));
    }
    Ok(nsurl)
}
