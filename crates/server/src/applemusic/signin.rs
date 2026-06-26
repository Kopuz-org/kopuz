use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use config::Browser;

use crate::cookies::browser as ip;

const SIGNIN_URL: &str = "https://music.apple.com/signin";
const COOKIE_DOMAIN: &str = "music.apple.com";
pub fn profile_dir(server_id: &str) -> PathBuf {
    let safe: String = server_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .collect();
    let leaf = if safe.is_empty() {
        "am-profile".to_string()
    } else {
        format!("am-profile-{safe}")
    };
    directories::ProjectDirs::from("com", "temidaradev", "kopuz")
        .map(|d| {
            #[cfg(target_os = "windows")]
            let base = d.data_local_dir();
            #[cfg(not(target_os = "windows"))]
            let base = d.config_dir();
            base.join(&leaf)
        })
        .unwrap_or_else(|| PathBuf::from(format!("./{leaf}")))
}

pub fn delete_profile(server_id: &str) -> std::io::Result<()> {
    match std::fs::remove_dir_all(profile_dir(server_id)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[tracing::instrument(name = "am.signin", skip(server_id, signin_timeout), fields(browser = %browser))]
pub async fn launch_signin_and_extract(
    browser: Browser,
    server_id: &str,
    signin_timeout: Duration,
) -> Result<String, String> {
    let profile = profile_dir(server_id);
    match tokio::fs::remove_dir_all(&profile).await {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("wipe am-profile: {e}")),
    }
    tokio::fs::create_dir_all(&profile)
        .await
        .map_err(|e| format!("mkdir am-profile: {e}"))?;

    let bin = if ip::in_flatpak() {
        ip::find_host_browser_bin(browser).await.ok_or_else(|| {
            format!(
                "{browser} not found on the host (looked for: {}). Install it on the host system.",
                ip::browser_candidates(browser).join(", ")
            )
        })?
    } else {
        ip::find_browser_bin(browser).ok_or_else(|| {
            format!(
                "{browser} not found in PATH (looked for: {}). Install it, or set $KOPUZ_{}_BIN.",
                ip::browser_candidates(browser).join(", "),
                browser.id().to_uppercase().replace('-', "_")
            )
        })?
    };

    let mut cmd = ip::browser_command(&bin);
    cmd.arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg(format!("--user-data-dir={}", profile.display()));
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0100_0000);
    }
    let mut child = cmd
        .arg(SIGNIN_URL)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("spawn {bin}: {e}"))?;

    let deadline = Instant::now() + signin_timeout;
    let outcome = loop {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if Instant::now() > deadline {
            break Err(format!(
                "timed out after {}s — sign in at {} and try again",
                signin_timeout.as_secs(),
                SIGNIN_URL
            ));
        }
        if let Some(t) = extract_media_user_token(browser, &profile).await {
            break Ok(t);
        }
        let _ = child.try_wait();
    };
    drop(child);
    outcome
}

async fn extract_media_user_token(browser: Browser, profile: &Path) -> Option<String> {
    tracing::debug!("am.signin.extract: looking for media-user-token cookie");
    let result = extract_cookie(browser, profile, "media-user-token")
        .await
        .ok()
        .flatten();
    if result.is_some() {
        tracing::debug!("am.signin.extract: found media-user-token");
    } else {
        tracing::debug!("am.signin.extract: media-user-token not yet present");
    }
    result
}

pub async fn extract_cookie(
    browser: Browser,
    profile_root: &Path,
    name: &str,
) -> Result<Option<String>, String> {
    let db_path = pick_cookies_path(profile_root).ok_or_else(|| {
        tracing::warn!(
            "am.signin.extract_cookie: no Cookies database at {}",
            profile_root.display()
        );
        "no Cookies database yet".to_string()
    })?;
    let browser_name = rookie_browser_name(browser);
    tracing::debug!(
        "am.signin.extract_cookie: name={name}, db={}",
        db_path.display()
    );
    let profile_owned = profile_root.to_path_buf();

    let cookies =
        tokio::task::spawn_blocking(move || -> Result<Vec<rookie::enums::Cookie>, String> {
            let domains = Some(vec![COOKIE_DOMAIN.to_string()]);
            #[cfg(not(target_os = "windows"))]
            {
                let _ = profile_owned;
                let config = rookie::config::get_browser_config(browser_name);
                rookie::chromium_based(config, db_path, domains).map_err(|e| e.to_string())
            }
            #[cfg(target_os = "windows")]
            {
                let _ = browser_name;
                let key_path = profile_owned.join("Local State");
                rookie::chromium_based(key_path, db_path, domains).map_err(|e| e.to_string())
            }
        })
        .await
        .map_err(|e| format!("cookie extract task: {e}"))??;

    tracing::debug!(
        "am.signin.extract_cookie: {} cookies from {}",
        cookies.len(),
        COOKIE_DOMAIN
    );
    let found = cookies
        .into_iter()
        .find(|c| c.name == name && !c.value.is_empty())
        .map(|c| c.value);
    if found.is_none() {
        tracing::debug!("am.signin.extract_cookie: cookie '{name}' not found or empty");
    }
    Ok(found)
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
    [
        profile_root.join("Default").join("Network").join("Cookies"),
        profile_root.join("Default").join("Cookies"),
    ]
    .into_iter()
    .find(|p| p.exists())
}
