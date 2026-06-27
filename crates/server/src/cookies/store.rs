use std::path::Path;

use config::Browser;

/// A decrypted cookie — kopuz's consumers (YT Music + SoundCloud header
/// builders) only ever read `name`/`value`, so this stays minimal and works on
/// every platform (the non-Windows backend maps `rookie`'s richer struct down
/// to it; Windows produces it natively).
#[derive(Debug, Clone)]
pub(crate) struct Cookie {
    pub name: String,
    pub value: String,
}

/// Decrypt the isolated profile's Chromium cookie store (via `rookie`) and
/// return every cookie scoped to `domain`.
#[cfg(not(target_os = "windows"))]
pub(crate) async fn read_cookies(
    browser: Browser,
    profile_root: &Path,
    domain: &str,
) -> Result<Vec<Cookie>, String> {
    let db_path = super::profile::pick_cookies_path(profile_root).ok_or_else(|| {
        format!(
            "no Cookies database under {} — is `{}` installed?",
            profile_root.display(),
            browser.label()
        )
    })?;
    let browser_name = browser.id();
    let domains = vec![domain.to_string()];

    let cookies = tokio::task::spawn_blocking(move || -> Result<Vec<Cookie>, String> {
        let config = rookie::config::get_browser_config(browser_name);
        let raw = rookie::chromium_based(config, db_path, Some(domains)).map_err(|e| e.to_string())?;
        Ok(raw
            .into_iter()
            .map(|c| Cookie {
                name: c.name,
                value: c.value,
            })
            .collect())
    })
    .await
    .map_err(|e| format!("cookie extract task: {e}"))??;
    tracing::trace!(browser = browser_name, domain, count = cookies.len(), "read cookies from isolated profile");
    Ok(cookies)
}

/// Windows: native v10/v11 (DPAPI) + v20 (planted app-bound) decryption — no
/// `rookie`/`libesedb`, no admin. See [`super::windows_native`].
#[cfg(target_os = "windows")]
pub(crate) async fn read_cookies(
    browser: Browser,
    profile_root: &Path,
    domain: &str,
) -> Result<Vec<Cookie>, String> {
    super::windows_native::read_cookies(browser, profile_root, domain).await
}
