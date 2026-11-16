//! In-app review prompt for App Store / Google Play Store.
//!
//! Provides a cross-platform API to request in-app reviews backed by:
//! - Android: Google Play Store intents
//! - iOS: SKStoreReviewController
//!
//! Feature-gated behind `in_app_review`.

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "ios")]
mod ios;

/// Check if the in-app review flow is available on this device.
pub fn is_available() -> Result<bool, String> {
    #[cfg(target_os = "ios")]
    {
        ios::is_available()
    }
    #[cfg(target_os = "android")]
    {
        android::is_available()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        Err("in_app_review is only available on iOS and Android".into())
    }
}

/// Request the in-app review flow.
/// Note: The OS may choose not to show the review dialog (rate limiting).
/// There is no way to know if the dialog was actually shown.
pub fn request_review() -> Result<(), String> {
    #[cfg(target_os = "ios")]
    {
        ios::request_review()
    }
    #[cfg(target_os = "android")]
    {
        android::request_review()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        Err("in_app_review is only available on iOS and Android".into())
    }
}

/// Open the app's store listing page directly.
/// `app_id` is the app's ID (e.g. package name on Android, Apple ID on iOS).
pub fn open_store_listing(app_id: &str) -> Result<(), String> {
    #[cfg(target_os = "ios")]
    {
        ios::open_store_listing(app_id)
    }
    #[cfg(target_os = "android")]
    {
        android::open_store_listing(app_id)
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        let _ = app_id;
        Err("in_app_review is only available on iOS and Android".into())
    }
}
