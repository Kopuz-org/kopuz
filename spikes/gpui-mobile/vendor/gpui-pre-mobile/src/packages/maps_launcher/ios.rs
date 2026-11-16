use objc2::runtime::AnyObject;
use objc2::{class, msg_send};

pub fn open_coordinates(
    latitude: f64,
    longitude: f64,
    label: Option<&str>,
) -> Result<bool, String> {
    let url = match label {
        Some(l) => format!(
            "http://maps.apple.com/?ll={},{}&q={}",
            latitude,
            longitude,
            percent_encode(l)
        ),
        None => format!("http://maps.apple.com/?ll={},{}", latitude, longitude),
    };
    open_url(&url)
}

pub fn open_query(query: &str) -> Result<bool, String> {
    let url = format!("http://maps.apple.com/?q={}", percent_encode(query));
    open_url(&url)
}

pub fn open_directions(
    dest_latitude: f64,
    dest_longitude: f64,
    dest_label: Option<&str>,
) -> Result<bool, String> {
    let url = match dest_label {
        Some(l) => format!(
            "http://maps.apple.com/?daddr={},{}&q={}&dirflg=d",
            dest_latitude,
            dest_longitude,
            percent_encode(l)
        ),
        None => format!(
            "http://maps.apple.com/?daddr={},{}&dirflg=d",
            dest_latitude, dest_longitude
        ),
    };
    open_url(&url)
}

pub fn is_available() -> Result<bool, String> {
    // Apple Maps is always available on iOS.
    Ok(true)
}

fn open_url(url_string: &str) -> Result<bool, String> {
    unsafe {
        let nsurl = nsurl_from_str(url_string)?;
        let app: *mut AnyObject = msg_send![class!(UIApplication), sharedApplication];
        if app.is_null() {
            return Err("Failed to get UIApplication.sharedApplication".into());
        }
        let can_open: bool = msg_send![app, canOpenURL: nsurl];
        if can_open {
            let _: () = msg_send![app, openURL: nsurl];
            Ok(true)
        } else {
            Ok(false)
        }
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
    if nsurl.is_null() {
        return Err(format!("Failed to parse URL: {url}"));
    }
    Ok(nsurl)
}

/// Simple percent-encoding for URL query parameters.
fn percent_encode(input: &str) -> String {
    let mut result = String::with_capacity(input.len() * 2);
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            b' ' => {
                result.push_str("%20");
            }
            _ => {
                result.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    result
}
