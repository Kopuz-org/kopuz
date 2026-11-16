//! URL launching (open URLs in the system browser or other apps).

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "ios")]
mod ios;

/// Attempt to launch the given URL.
///
/// Returns `Ok(true)` if the URL was opened successfully, `Ok(false)` if the
/// system could not handle the URL scheme.
pub fn launch_url(url: &str) -> Result<bool, String> {
    #[cfg(target_os = "ios")]
    {
        ios::launch_url(url)
    }
    #[cfg(target_os = "android")]
    {
        android::launch_url(url)
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        let _ = url;
        Err("url_launcher is only available on iOS and Android".into())
    }
}

/// Check whether the given URL can be handled by an installed app.
pub fn can_launch_url(url: &str) -> Result<bool, String> {
    #[cfg(target_os = "ios")]
    {
        ios::can_launch_url(url)
    }
    #[cfg(target_os = "android")]
    {
        android::can_launch_url(url)
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        let _ = url;
        Err("url_launcher is only available on iOS and Android".into())
    }
}
