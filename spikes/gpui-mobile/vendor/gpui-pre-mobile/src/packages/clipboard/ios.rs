use objc2::runtime::AnyObject;
use objc2::{class, msg_send};

pub fn set_text(text: &str) -> Result<(), String> {
    unsafe {
        let pasteboard: *mut AnyObject = msg_send![class!(UIPasteboard), generalPasteboard];
        if pasteboard.is_null() {
            return Err("Failed to get UIPasteboard".into());
        }

        let ns_text: *mut AnyObject = msg_send![class!(NSString), alloc];
        let ns_text: *mut AnyObject = msg_send![ns_text,
            initWithBytes: text.as_ptr() as *const std::ffi::c_void,
            length: text.len(),
            encoding: 4u64  // NSUTF8StringEncoding
        ];
        if ns_text.is_null() {
            return Err("Failed to create NSString".into());
        }

        let _: () = msg_send![pasteboard, setString: ns_text];
        Ok(())
    }
}

pub fn get_text() -> Result<Option<String>, String> {
    unsafe {
        let pasteboard: *mut AnyObject = msg_send![class!(UIPasteboard), generalPasteboard];
        if pasteboard.is_null() {
            return Err("Failed to get UIPasteboard".into());
        }

        let has: bool = msg_send![pasteboard, hasStrings];
        if !has {
            return Ok(None);
        }

        let ns_string: *mut AnyObject = msg_send![pasteboard, string];
        if ns_string.is_null() {
            return Ok(None);
        }

        let utf8: *const std::ffi::c_char = msg_send![ns_string, UTF8String];
        if utf8.is_null() {
            return Ok(None);
        }

        let c_str = std::ffi::CStr::from_ptr(utf8);
        match c_str.to_str() {
            Ok(s) => Ok(Some(s.to_owned())),
            Err(_) => Ok(Some(c_str.to_string_lossy().into_owned())),
        }
    }
}

pub fn has_text() -> Result<bool, String> {
    unsafe {
        let pasteboard: *mut AnyObject = msg_send![class!(UIPasteboard), generalPasteboard];
        if pasteboard.is_null() {
            return Err("Failed to get UIPasteboard".into());
        }

        let has: bool = msg_send![pasteboard, hasStrings];
        Ok(has)
    }
}
