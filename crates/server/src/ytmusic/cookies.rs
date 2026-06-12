//! Cookie reader for the isolated YT Music profile. Delegates the
//! platform-specific decryption (libsecret on Linux, Keychain on
//! macOS, DPAPI on Windows) to the `rookie` crate. Our wrapper picks
//! the right preset config per browser and points rookie at the
//! `~/.config/kopuz/yt-profile-<id>/Default/Cookies` we own.

use std::path::{Path, PathBuf};

use config::Browser;

/// Extract YouTube cookies from `profile_root` (an isolated kopuz
/// profile, not the user's main browser). Returns a `Cookie:` header.
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
    let profile_root_owned = profile_root.to_path_buf();

    let cookies =
        tokio::task::spawn_blocking(move || -> Result<Vec<rookie::enums::Cookie>, String> {
            let domains = Some(vec!["youtube.com".to_string()]);
            #[cfg(not(target_os = "windows"))]
            {
                let _ = profile_root_owned;
                let config = rookie::config::get_browser_config(browser_name);
                rookie::chromium_based(config, db_path, domains).map_err(|e| e.to_string())
            }
            #[cfg(target_os = "windows")]
            {
                let _ = browser_name;
                let key_path = profile_root_owned.join("Local State");
                rookie::chromium_based(key_path, db_path, domains).map_err(|e| e.to_string())
            }
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

fn rookie_browser_name(browser: Browser) -> &'static str {
    match browser {
        Browser::Brave => "brave",
        Browser::Chrome => "chrome",
        Browser::Chromium => "chromium",
        Browser::Edge => "edge",
        Browser::Vivaldi => "vivaldi",
    }
}

fn pick_cookies_path(profile_root: &Path) -> Option<PathBuf> {
    let candidates = [
        profile_root.join("Default").join("Network").join("Cookies"),
        profile_root.join("Default").join("Cookies"),
    ];
    candidates.into_iter().find(|p| p.exists())
}

fn header_safe(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| (0x20..0x7f).contains(&b) && b != b';' && b != b',')
}
