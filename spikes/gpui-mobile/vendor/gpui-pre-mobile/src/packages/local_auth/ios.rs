use super::{AuthResult, BiometricType};
use objc2::runtime::{AnyObject, Bool};
use objc2::{class, msg_send};
use std::ffi::c_void;

/// LAPolicy constants.
const LA_POLICY_DEVICE_OWNER_AUTHENTICATION_WITH_BIOMETRICS: i64 = 1;

/// LABiometryType constants.
const LA_BIOMETRY_TYPE_NONE: i64 = 0;
const LA_BIOMETRY_TYPE_TOUCH_ID: i64 = 1;
const LA_BIOMETRY_TYPE_FACE_ID: i64 = 2;

/// LAError constants.
const LA_ERROR_AUTHENTICATION_FAILED: i64 = -1;
const LA_ERROR_USER_CANCEL: i64 = -2;
const LA_ERROR_SYSTEM_CANCEL: i64 = -4;
const LA_ERROR_PASSCODE_NOT_SET: i64 = -5;
const LA_ERROR_BIOMETRY_NOT_AVAILABLE: i64 = -6;
const LA_ERROR_BIOMETRY_NOT_ENROLLED: i64 = -7;
const LA_ERROR_BIOMETRY_LOCKOUT: i64 = -8;

unsafe fn create_la_context() -> *mut AnyObject {
    let context: *mut AnyObject = msg_send![class!(LAContext), alloc];
    let context: *mut AnyObject = msg_send![context, init];
    context
}

pub fn is_device_supported() -> Result<bool, String> {
    unsafe {
        let context = create_la_context();
        if context.is_null() {
            return Err("Failed to create LAContext".into());
        }

        // canEvaluatePolicy:error: — pass nil for error pointer
        let can_evaluate: bool = msg_send![
            context,
            canEvaluatePolicy: LA_POLICY_DEVICE_OWNER_AUTHENTICATION_WITH_BIOMETRICS,
            error: std::ptr::null_mut::<*mut AnyObject>()
        ];

        // Even if canEvaluatePolicy returns false (e.g. not enrolled), the device
        // may still have hardware. Check biometryType.
        let biometry_type: i64 = msg_send![context, biometryType];
        let _: () = msg_send![context, release];

        Ok(can_evaluate || biometry_type != LA_BIOMETRY_TYPE_NONE)
    }
}

pub fn can_authenticate() -> Result<bool, String> {
    unsafe {
        let context = create_la_context();
        if context.is_null() {
            return Err("Failed to create LAContext".into());
        }

        let can_evaluate: bool = msg_send![
            context,
            canEvaluatePolicy: LA_POLICY_DEVICE_OWNER_AUTHENTICATION_WITH_BIOMETRICS,
            error: std::ptr::null_mut::<*mut AnyObject>()
        ];

        let _: () = msg_send![context, release];
        Ok(can_evaluate)
    }
}

pub fn get_available_biometrics() -> Result<Vec<BiometricType>, String> {
    unsafe {
        let context = create_la_context();
        if context.is_null() {
            return Err("Failed to create LAContext".into());
        }

        // Must call canEvaluatePolicy first to populate biometryType
        let _can: bool = msg_send![
            context,
            canEvaluatePolicy: LA_POLICY_DEVICE_OWNER_AUTHENTICATION_WITH_BIOMETRICS,
            error: std::ptr::null_mut::<*mut AnyObject>()
        ];

        let biometry_type: i64 = msg_send![context, biometryType];
        let _: () = msg_send![context, release];

        let mut types = Vec::new();
        match biometry_type {
            LA_BIOMETRY_TYPE_TOUCH_ID => types.push(BiometricType::Fingerprint),
            LA_BIOMETRY_TYPE_FACE_ID => types.push(BiometricType::Face),
            _ => {}
        }

        Ok(types)
    }
}

pub fn authenticate(reason: &str) -> Result<AuthResult, String> {
    unsafe {
        let context = create_la_context();
        if context.is_null() {
            return Err("Failed to create LAContext".into());
        }

        // Check if we can authenticate first
        let can_evaluate: bool = msg_send![
            context,
            canEvaluatePolicy: LA_POLICY_DEVICE_OWNER_AUTHENTICATION_WITH_BIOMETRICS,
            error: std::ptr::null_mut::<*mut AnyObject>()
        ];

        if !can_evaluate {
            let biometry_type: i64 = msg_send![context, biometryType];
            let _: () = msg_send![context, release];
            return Ok(if biometry_type == LA_BIOMETRY_TYPE_NONE {
                AuthResult::ErrorNotAvailable
            } else {
                AuthResult::ErrorNotEnrolled
            });
        }

        // Create NSString for reason
        let reason_nsstring: *mut AnyObject = msg_send![class!(NSString), alloc];
        let reason_bytes = reason.as_bytes();
        let reason_nsstring: *mut AnyObject = msg_send![
            reason_nsstring,
            initWithBytes: reason_bytes.as_ptr() as *const c_void,
            length: reason_bytes.len(),
            encoding: 4u64 // NSUTF8StringEncoding
        ];

        // Use a dispatch semaphore to block until the callback fires
        let semaphore = dispatch_semaphore_create(0);

        // We need to capture the result from the callback.
        // Use a Box<std::sync::Mutex<AuthResult>> shared via raw pointer.
        let result_holder = Box::into_raw(Box::new(std::sync::Mutex::new(AuthResult::ErrorOther)));

        // Create the reply block.
        // The block signature is: void (^)(BOOL success, NSError *error)
        let block = block2::RcBlock::new(move |success: Bool, error: *mut AnyObject| {
            let auth_result = if success.as_bool() {
                AuthResult::Success
            } else if error.is_null() {
                AuthResult::Failed
            } else {
                let error_code: i64 = msg_send![error, code];
                match error_code {
                    LA_ERROR_AUTHENTICATION_FAILED => AuthResult::Failed,
                    LA_ERROR_USER_CANCEL => AuthResult::ErrorUserCancelled,
                    LA_ERROR_SYSTEM_CANCEL => AuthResult::ErrorUserCancelled,
                    LA_ERROR_PASSCODE_NOT_SET => AuthResult::ErrorPasscodeNotSet,
                    LA_ERROR_BIOMETRY_NOT_AVAILABLE => AuthResult::ErrorNotAvailable,
                    LA_ERROR_BIOMETRY_NOT_ENROLLED => AuthResult::ErrorNotEnrolled,
                    LA_ERROR_BIOMETRY_LOCKOUT => AuthResult::ErrorLockout,
                    _ => AuthResult::ErrorOther,
                }
            };

            // Store the result
            if let Ok(mut guard) = (*result_holder).lock() {
                *guard = auth_result;
            }

            dispatch_semaphore_signal(semaphore);
        });

        // Call evaluatePolicy:localizedReason:reply:
        let _: () = msg_send![
            context,
            evaluatePolicy: LA_POLICY_DEVICE_OWNER_AUTHENTICATION_WITH_BIOMETRICS,
            localizedReason: reason_nsstring,
            reply: &*block as *const _ as *const c_void
        ];

        // Wait for the callback
        dispatch_semaphore_wait(semaphore, DISPATCH_TIME_FOREVER);

        // Read the result
        let result_box = Box::from_raw(result_holder);
        let auth_result = result_box
            .lock()
            .map(|g| *g)
            .unwrap_or(AuthResult::ErrorOther);

        let _: () = msg_send![reason_nsstring, release];
        let _: () = msg_send![context, release];
        dispatch_release(semaphore as *mut c_void);

        Ok(auth_result)
    }
}

// Grand Central Dispatch C functions
const DISPATCH_TIME_FOREVER: u64 = !0;

extern "C" {
    fn dispatch_semaphore_create(value: i64) -> *mut AnyObject;
    fn dispatch_semaphore_signal(dsema: *mut AnyObject) -> i64;
    fn dispatch_semaphore_wait(dsema: *mut AnyObject, timeout: u64) -> i64;
    fn dispatch_release(object: *mut c_void);
}
