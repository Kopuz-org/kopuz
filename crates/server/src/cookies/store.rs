use std::path::Path;

use config::Browser;

/// Decrypt the isolated profile's Chromium cookie store (via `rookie`) and
/// return every cookie scoped to `domain`.
#[cfg(not(target_os = "windows"))]
pub(crate) async fn read_cookies(
    browser: Browser,
    profile_root: &Path,
    domain: &str,
) -> Result<Vec<rookie::enums::Cookie>, String> {
    let db_path = super::profile::pick_cookies_path(profile_root).ok_or_else(|| {
        format!(
            "no Cookies database under {} — is `{}` installed?",
            profile_root.display(),
            browser.label()
        )
    })?;
    let browser_name = browser.id();
    let domains = vec![domain.to_string()];

    let cookies =
        tokio::task::spawn_blocking(move || -> Result<Vec<rookie::enums::Cookie>, String> {
            let config = rookie::config::get_browser_config(browser_name);
            rookie::chromium_based(config, db_path, Some(domains)).map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| format!("cookie extract task: {e}"))??;
    tracing::trace!(
        browser = browser_name,
        domain,
        count = cookies.len(),
        "read cookies from isolated profile"
    );
    Ok(cookies)
}

/// Windows: unsupported — Chromium v20's App-Bound Encryption blocks non-admin
/// decryption, and `rookie`'s ESE reader (`libesedb`) isn't built there.
#[cfg(target_os = "windows")]
pub(crate) async fn read_cookies(
    browser: Browser,
    profile_root: &Path,
    domain: &str,
) -> Result<Vec<rookie::enums::Cookie>, String> {
    let _ = (browser, profile_root, domain);
    Err("browser-cookie import isn't supported on Windows".to_string())
}
