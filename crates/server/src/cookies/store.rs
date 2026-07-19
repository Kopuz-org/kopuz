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
        // rookie's built-in table has no `helium` entry and `get_browser_config`
        // unwraps on a miss, so build Helium's config by hand. As an
        // ungoogled-chromium fork its `Safe Storage` secret is keyed by the
        // "Helium" product name (login Keychain on macOS, "Helium Safe Storage"
        // libsecret label on Linux) — not Chromium's.
        let helium_config;
        let config = match browser {
            Browser::Helium => {
                helium_config = rookie::config::Browser {
                    paths: Vec::new(),
                    channels: None,
                    unix_crypt_name: Some("helium".to_string()),
                    osx_key_service: Some("Helium Safe Storage".to_string()),
                    osx_key_user: Some("Helium".to_string()),
                };
                &helium_config
            }
            _ => rookie::config::get_browser_config(browser_name),
        };
        let raw =
            rookie::chromium_based(config, db_path, Some(domains)).map_err(|e| e.to_string())?;
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
    tracing::trace!(
        browser = browser_name,
        domain,
        count = cookies.len(),
        "read cookies from isolated profile"
    );
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
