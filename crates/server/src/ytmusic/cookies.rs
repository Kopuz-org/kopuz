//! Cookie reader for the isolated YT Music profile. Delegates the
//! platform-specific decryption (libsecret on Linux, Keychain on
//! macOS, DPAPI on Windows) to the `rookie` crate. Our wrapper picks
//! the right preset config per browser and points rookie at the
//! `~/.config/kopuz/yt-profile-<id>/Default/Cookies` we own.

use std::path::Path;
#[cfg(not(target_os = "windows"))]
use std::path::PathBuf;

use config::Browser;

/// Extract YouTube cookies from `profile_root` (an isolated kopuz
/// profile, not the user's main browser). Returns a `Cookie:` header.
#[cfg(not(target_os = "windows"))]
#[tracing::instrument(name = "yt.cookies_extract", skip(profile_root), fields(browser = %browser))]
pub async fn extract_from(browser: Browser, profile_root: &Path) -> Result<String, String> {
    let db_path = pick_cookies_path(profile_root).ok_or_else(|| {
        format!(
            "no Cookies database under {} — is `{}` installed?",
            profile_root.display(),
            browser.label()
        )
    })?;

    let browser_name = rookie_browser_name(browser);

    let cookies =
        tokio::task::spawn_blocking(move || -> Result<Vec<rookie::enums::Cookie>, String> {
            let domains = Some(vec!["youtube.com".to_string()]);
            let config = rookie::config::get_browser_config(browser_name);
            rookie::chromium_based(config, db_path, domains).map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| format!("cookie extract task: {e}"))??;

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

/// Windows: browser-cookie import is unsupported — Chromium v20's App-Bound
/// Encryption blocks non-admin cookie decryption, and `rookie`'s ESE reader
/// (`libesedb`) isn't built there. Callers fall back to anonymous access.
#[cfg(target_os = "windows")]
pub async fn extract_from(browser: Browser, profile_root: &Path) -> Result<String, String> {
    let _ = (browser, profile_root);
    Err("browser-cookie import isn't supported on Windows".to_string())
}

#[cfg(not(target_os = "windows"))]
fn rookie_browser_name(browser: Browser) -> &'static str {
    match browser {
        Browser::Brave => "brave",
        Browser::Chrome => "chrome",
        Browser::Chromium => "chromium",
        Browser::Edge => "edge",
        Browser::Vivaldi => "vivaldi",
    }
}

#[cfg(not(target_os = "windows"))]
fn pick_cookies_path(profile_root: &Path) -> Option<PathBuf> {
    let candidates = [
        profile_root.join("Default").join("Network").join("Cookies"),
        profile_root.join("Default").join("Cookies"),
    ];
    candidates.into_iter().find(|p| p.exists())
}

#[cfg(not(target_os = "windows"))]
fn header_safe(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| (0x20..0x7f).contains(&b) && b != b';' && b != b',')
}
