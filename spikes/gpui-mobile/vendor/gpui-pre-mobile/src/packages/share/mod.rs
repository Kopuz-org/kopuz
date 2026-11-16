//! Share text, URIs, and files via the platform share sheet.

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "ios")]
mod ios;

/// Result of a share action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareResult {
    /// User completed the share.
    Success,
    /// User dismissed the share sheet.
    Dismissed,
    /// The platform could not determine the result.
    Unavailable,
}

/// Share text content via the platform share sheet.
///
/// `text` — the content to share (can be a message, URL, or any string).
/// `subject` — optional subject line (used by email and some apps).
pub fn share_text(text: &str, subject: Option<&str>) -> Result<(), String> {
    #[cfg(target_os = "ios")]
    {
        ios::share_text(text, subject)
    }
    #[cfg(target_os = "android")]
    {
        android::share_text(text, subject)
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        let _ = (text, subject);
        Err("share is only available on iOS and Android".into())
    }
}

/// Share a URI via the platform share sheet.
pub fn share_uri(uri: &str) -> Result<(), String> {
    share_text(uri, None)
}
