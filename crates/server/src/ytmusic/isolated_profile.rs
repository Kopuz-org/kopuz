//! One-time YouTube Music sign-in via the shared [`crate::cookies`] isolated
//! browser-profile flow, pointed at Google's ServiceLogin. From there
//! [`super::verify_session_keepalive`] keeps the session alive over HTTP.

use std::path::PathBuf;
use std::time::Duration;

use config::Browser;

use crate::cookies;

const SIGNIN_URL: &str = "https://accounts.google.com/ServiceLogin?service=youtube&continue=https%3A%2F%2Fmusic.youtube.com%2F";
const PROFILE_PREFIX: &str = "yt-profile";

pub fn profile_dir(server_id: &str) -> PathBuf {
    cookies::profile_dir(PROFILE_PREFIX, server_id)
}

pub fn delete_profile(server_id: &str) -> std::io::Result<()> {
    cookies::delete_profile(PROFILE_PREFIX, server_id)
}

// Windows browser sign-in is now supported: cookies are decrypted natively
// (v10 DPAPI + planted v20 app-bound key — see `crate::cookies::windows_native`),
// so the extract callback below resolves the same as Linux/macOS.
#[tracing::instrument(name = "yt.signin", skip(server_id, signin_timeout), fields(browser = %browser))]
pub async fn launch_signin_and_extract(
    browser: Browser,
    server_id: &str,
    signin_timeout: Duration,
) -> Result<String, String> {
    cookies::launch_signin_and_extract(
        browser,
        server_id,
        PROFILE_PREFIX,
        SIGNIN_URL,
        signin_timeout,
        |browser, profile| async move {
            let header = super::cookies::extract_from(browser, &profile).await?;
            if cookies::has_cookie(&header, "SAPISID") && cookies::has_cookie(&header, "SID") {
                Ok(Some(header))
            } else {
                Ok(None)
            }
        },
    )
    .await
}
