use std::path::Path;

use config::{Browser, BrowserEngine};

/// A decrypted cookie — kopuz's consumers (YT Music + SoundCloud header
/// builders) only ever read `name`/`value`, so this stays minimal and works on
/// every platform (the non-Windows backend maps `rookie`'s richer struct down
/// to it; Windows produces it natively).
#[derive(Debug, Clone)]
pub(crate) struct Cookie {
    pub name: String,
    pub value: String,
}

/// A cookie `host_key` belongs to `domain` only as the domain itself or a
/// dot-prefixed subdomain — never a bare substring (`notyoutube.com`).
pub(crate) fn host_matches_domain(host: &str, domain: &str) -> bool {
    let host = host.strip_prefix('.').unwrap_or(host);
    host == domain || host.ends_with(&format!(".{domain}"))
}

/// Read every cookie scoped to `domain` out of the isolated profile. Gecko
/// keeps its store in the clear, so only the Chromium path needs decrypting.
pub(crate) async fn read_cookies(
    browser: Browser,
    profile_root: &Path,
    domain: &str,
) -> Result<Vec<Cookie>, String> {
    match browser.engine() {
        BrowserEngine::Chromium => read_chromium_cookies(browser, profile_root, domain).await,
        BrowserEngine::Gecko => read_gecko_cookies(browser, profile_root, domain).await,
    }
}

#[cfg(not(target_os = "android"))]
async fn read_gecko_cookies(
    browser: Browser,
    profile_root: &Path,
    domain: &str,
) -> Result<Vec<Cookie>, String> {
    super::mozilla::read_cookies(browser, profile_root, domain).await
}

#[cfg(target_os = "android")]
async fn read_gecko_cookies(
    browser: Browser,
    _profile_root: &Path,
    _domain: &str,
) -> Result<Vec<Cookie>, String> {
    Err(format!(
        "browser cookie import is desktop-only ({})",
        browser.label()
    ))
}

/// Decrypt the isolated profile's Chromium cookie store (via `rookie`) and
/// return every cookie scoped to `domain`.
#[cfg(not(any(target_os = "windows", target_os = "android")))]
async fn read_chromium_cookies(
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
        // ungoogled-chromium fork Helium only rebrands its `Safe Storage` secret
        // on macOS ("Helium Safe Storage" in the login Keychain). Its Linux
        // build keeps the upstream os_crypt product name, so cookies are keyed
        // by the "Chromium Safe Storage" libsecret label (application attribute
        // "chromium") — using "helium" there finds no key, so the v11 cookies
        // never decrypt and sign-in polls forever.
        let helium_config;
        let config = match browser {
            Browser::Helium => {
                helium_config = rookie::config::Browser {
                    paths: Vec::new(),
                    channels: None,
                    unix_crypt_name: Some("chromium".to_string()),
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

/// Android: the isolated-profile sign-in that fills a Chromium cookie store is
/// a desktop flow, so there is never a profile here to read.
#[cfg(target_os = "android")]
async fn read_chromium_cookies(
    browser: Browser,
    _profile_root: &Path,
    _domain: &str,
) -> Result<Vec<Cookie>, String> {
    Err(format!(
        "browser cookie import is desktop-only ({})",
        browser.label()
    ))
}

/// Windows: native v10/v11 (DPAPI) + v20 (planted app-bound) decryption — no
/// `rookie`/`libesedb`, no admin. See [`super::windows_native`].
#[cfg(target_os = "windows")]
async fn read_chromium_cookies(
    browser: Browser,
    profile_root: &Path,
    domain: &str,
) -> Result<Vec<Cookie>, String> {
    super::windows_native::read_cookies(browser, profile_root, domain).await
}

#[cfg(test)]
mod tests {
    use super::host_matches_domain;

    #[test]
    fn a_domain_cookie_is_not_matched_by_a_lookalike_host() {
        assert!(host_matches_domain("youtube.com", "youtube.com"));
        assert!(host_matches_domain(".youtube.com", "youtube.com"));
        assert!(host_matches_domain("music.youtube.com", "youtube.com"));
        assert!(!host_matches_domain("notyoutube.com", "youtube.com"));
        assert!(!host_matches_domain("youtube.com.evil.test", "youtube.com"));
    }
}
