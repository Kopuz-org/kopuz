use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use std::ffi::CStr;

pub struct IosSharedPreferences;

impl IosSharedPreferences {
    pub fn new() -> Self {
        Self
    }

    pub fn get_string(&self, key: &str) -> Option<String> {
        unsafe {
            let defaults = user_defaults();
            let ns_key = nsstring(key);
            let value: *mut AnyObject = msg_send![defaults, stringForKey: ns_key];
            if value.is_null() {
                None
            } else {
                Some(nsstring_to_string(value))
            }
        }
    }

    pub fn set_string(&self, key: &str, value: &str) -> Result<(), String> {
        unsafe {
            let defaults = user_defaults();
            let ns_key = nsstring(key);
            let ns_value = nsstring(value);
            let _: () = msg_send![defaults, setObject: ns_value, forKey: ns_key];
            let _: () = msg_send![defaults, synchronize];
            Ok(())
        }
    }

    pub fn get_int(&self, key: &str) -> Option<i64> {
        unsafe {
            let defaults = user_defaults();
            let ns_key = nsstring(key);
            let obj: *mut AnyObject = msg_send![defaults, objectForKey: ns_key];
            if obj.is_null() {
                None
            } else {
                let val: i64 = msg_send![defaults, integerForKey: ns_key];
                Some(val)
            }
        }
    }

    pub fn set_int(&self, key: &str, value: i64) -> Result<(), String> {
        unsafe {
            let defaults = user_defaults();
            let ns_key = nsstring(key);
            let _: () = msg_send![defaults, setInteger: value, forKey: ns_key];
            let _: () = msg_send![defaults, synchronize];
            Ok(())
        }
    }

    pub fn get_bool(&self, key: &str) -> Option<bool> {
        unsafe {
            let defaults = user_defaults();
            let ns_key = nsstring(key);
            let obj: *mut AnyObject = msg_send![defaults, objectForKey: ns_key];
            if obj.is_null() {
                None
            } else {
                let val: bool = msg_send![defaults, boolForKey: ns_key];
                Some(val)
            }
        }
    }

    pub fn set_bool(&self, key: &str, value: bool) -> Result<(), String> {
        unsafe {
            let defaults = user_defaults();
            let ns_key = nsstring(key);
            let _: () = msg_send![defaults, setBool: value, forKey: ns_key];
            let _: () = msg_send![defaults, synchronize];
            Ok(())
        }
    }

    pub fn remove(&self, key: &str) -> Result<(), String> {
        unsafe {
            let defaults = user_defaults();
            let ns_key = nsstring(key);
            let _: () = msg_send![defaults, removeObjectForKey: ns_key];
            let _: () = msg_send![defaults, synchronize];
            Ok(())
        }
    }

    pub fn clear(&self) -> Result<(), String> {
        unsafe {
            let defaults = user_defaults();
            let dict: *mut AnyObject = msg_send![defaults, dictionaryRepresentation];
            if dict.is_null() {
                return Ok(());
            }
            let keys: *mut AnyObject = msg_send![dict, allKeys];
            let count: u64 = msg_send![keys, count];
            for i in 0..count {
                let key: *mut AnyObject = msg_send![keys, objectAtIndex: i];
                let _: () = msg_send![defaults, removeObjectForKey: key];
            }
            let _: () = msg_send![defaults, synchronize];
            Ok(())
        }
    }

    pub fn contains_key(&self, key: &str) -> bool {
        unsafe {
            let defaults = user_defaults();
            let ns_key = nsstring(key);
            let obj: *mut AnyObject = msg_send![defaults, objectForKey: ns_key];
            !obj.is_null()
        }
    }
}

unsafe fn user_defaults() -> *mut AnyObject {
    msg_send![class!(NSUserDefaults), standardUserDefaults]
}

use crate::ios::util::nsstring;

unsafe fn nsstring_to_string(ns: *mut AnyObject) -> String {
    let utf8: *const i8 = msg_send![ns, UTF8String];
    if utf8.is_null() {
        return String::new();
    }
    CStr::from_ptr(utf8).to_string_lossy().into_owned()
}
