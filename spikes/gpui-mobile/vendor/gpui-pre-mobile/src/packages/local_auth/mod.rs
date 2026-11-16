//! Biometric authentication (fingerprint, face recognition).
//!
//! Provides a cross-platform local authentication API backed by:
//! - Android: BiometricPrompt via JNI
//! - iOS: LAContext (LocalAuthentication) via Objective-C
//!
//! Feature-gated behind `local_auth`.

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "ios")]
mod ios;

/// Types of biometric authentication available.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BiometricType {
    Fingerprint,
    Face,
    Iris,
}

/// Result of an authentication attempt.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AuthResult {
    Success,
    Failed,
    ErrorNotAvailable,
    ErrorNotEnrolled,
    ErrorUserCancelled,
    ErrorPasscodeNotSet,
    ErrorLockout,
    ErrorOther,
}

/// Check if the device supports biometric authentication.
pub fn is_device_supported() -> Result<bool, String> {
    #[cfg(target_os = "ios")]
    {
        ios::is_device_supported()
    }
    #[cfg(target_os = "android")]
    {
        android::is_device_supported()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        Ok(false)
    }
}

/// Check if biometrics are enrolled (configured) on the device.
pub fn can_authenticate() -> Result<bool, String> {
    #[cfg(target_os = "ios")]
    {
        ios::can_authenticate()
    }
    #[cfg(target_os = "android")]
    {
        android::can_authenticate()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        Ok(false)
    }
}

/// Get the list of available biometric types.
pub fn get_available_biometrics() -> Result<Vec<BiometricType>, String> {
    #[cfg(target_os = "ios")]
    {
        ios::get_available_biometrics()
    }
    #[cfg(target_os = "android")]
    {
        android::get_available_biometrics()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        Ok(vec![])
    }
}

/// Authenticate the user with biometrics.
/// `reason` is displayed to the user explaining why authentication is needed.
pub fn authenticate(reason: &str) -> Result<AuthResult, String> {
    #[cfg(target_os = "ios")]
    {
        ios::authenticate(reason)
    }
    #[cfg(target_os = "android")]
    {
        android::authenticate(reason)
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        let _ = reason;
        Ok(AuthResult::ErrorNotAvailable)
    }
}
