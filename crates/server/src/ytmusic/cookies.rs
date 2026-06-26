//! Cookie reader for the isolated YT Music profile: turns the cookies decrypted
//! by [`crate::cookies`] into the `Cookie:` header YT Music expects, requiring
//! the 1P auth cookies to be present.

use std::path::Path;

use config::Browser;

#[cfg(not(target_os = "windows"))]
#[tracing::instrument(name = "yt.cookies_extract", skip(profile_root), fields(browser = %browser))]
pub async fn extract_from(browser: Browser, profile_root: &Path) -> Result<String, String> {
    let cookies = crate::cookies::read_cookies(browser, profile_root, "youtube.com").await?;

    let header = cookies
        .iter()
        .filter(|c| !c.value.is_empty() && header_safe(&c.name) && header_safe(&c.value))
        .map(|c| format!("{}={}", c.name, c.value))
        .collect::<Vec<_>>()
        .join("; ");

    let has_auth = header.split(';').any(|p| {
        let Some((k, _)) = p.trim().split_once('=') else {
            return false;
        };
        k == "SAPISID" || k == "__Secure-3PAPISID"
    });
    if !has_auth {
        return Err(format!(
            "no auth cookies found in {} profile — sign in to YouTube Music there first",
            browser.label()
        ));
    }
    Ok(header)
}

/// Windows: unsupported — App-Bound Encryption + no `libesedb`; callers fall
/// back to anonymous access.
#[cfg(target_os = "windows")]
pub async fn extract_from(browser: Browser, profile_root: &Path) -> Result<String, String> {
    let _ = (browser, profile_root);
    Err("browser-cookie import isn't supported on Windows".to_string())
}

#[cfg(not(target_os = "windows"))]
fn header_safe(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| (0x20..0x7f).contains(&b) && b != b';' && b != b',')
}
