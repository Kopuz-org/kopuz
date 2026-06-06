//! Manages kopuz's isolated browser profile at
//! `~/.config/kopuz/yt-profile/`, used only for the one-time YouTube
//! Music sign-in. The user's real browser profile is never touched.
//!
//! Flow: wipe the profile dir → spawn `<browser>
//! --user-data-dir=<isolated>` pointed at Google ServiceLogin → poll
//! the profile's cookie SQLite until the 1P auth cookies appear →
//! kill the browser → return the decrypted cookie header. From there
//! [`super::verify_session_keepalive`] keeps the session alive over
//! HTTP without re-launching the browser.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use config::Browser;
use tokio::process::Command;

const SIGNIN_URL: &str =
    "https://accounts.google.com/ServiceLogin?service=youtube&continue=https%3A%2F%2Fmusic.youtube.com%2F";

pub fn profile_dir(server_id: &str) -> PathBuf {
    let safe: String = server_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .collect();
    let leaf = if safe.is_empty() {
        "yt-profile".to_string()
    } else {
        format!("yt-profile-{safe}")
    };
    directories::ProjectDirs::from("com", "temidaradev", "kopuz")
        .map(|d| d.config_dir().join(&leaf))
        .unwrap_or_else(|| PathBuf::from(format!("./{leaf}")))
}

fn browser_candidates(browser: Browser) -> &'static [&'static str] {
    match browser {
        Browser::Brave => &["brave", "brave-browser"],
        Browser::Chrome => &["google-chrome", "google-chrome-stable", "chrome"],
        Browser::Chromium => &["chromium", "chromium-browser"],
        Browser::Edge => &[
            "microsoft-edge",
            "microsoft-edge-stable",
            "microsoft-edge-beta",
            "microsoft-edge-dev",
        ],
        Browser::Vivaldi => &["vivaldi", "vivaldi-stable"],
    }
}

#[cfg(target_os = "macos")]
fn macos_app_paths(browser: Browser) -> &'static [&'static str] {
    match browser {
        Browser::Brave => &[
            "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
        ],
        Browser::Chrome => &[
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        ],
        Browser::Chromium => &[
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
        ],
        Browser::Edge => &[
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        ],
        Browser::Vivaldi => &[
            "/Applications/Vivaldi.app/Contents/MacOS/Vivaldi",
        ],
    }
}

#[cfg(target_os = "windows")]
fn windows_install_paths(browser: Browser) -> Vec<PathBuf> {
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let pf = env("ProgramFiles");
    let pf86 = env("ProgramFiles(x86)");
    let local = env("LOCALAPPDATA");
    let mut out = Vec::new();
    let mut add = |opt: &Option<PathBuf>, suffix: &str| {
        if let Some(base) = opt {
            out.push(base.join(suffix));
        }
    };
    match browser {
        Browser::Brave => {
            add(&pf, r"BraveSoftware\Brave-Browser\Application\brave.exe");
            add(&pf86, r"BraveSoftware\Brave-Browser\Application\brave.exe");
            add(&local, r"BraveSoftware\Brave-Browser\Application\brave.exe");
        }
        Browser::Chrome => {
            add(&pf, r"Google\Chrome\Application\chrome.exe");
            add(&pf86, r"Google\Chrome\Application\chrome.exe");
            add(&local, r"Google\Chrome\Application\chrome.exe");
        }
        Browser::Chromium => {
            add(&pf, r"Chromium\Application\chrome.exe");
            add(&pf86, r"Chromium\Application\chrome.exe");
            add(&local, r"Chromium\Application\chrome.exe");
        }
        Browser::Edge => {
            add(&pf, r"Microsoft\Edge\Application\msedge.exe");
            add(&pf86, r"Microsoft\Edge\Application\msedge.exe");
            add(&local, r"Microsoft\Edge\Application\msedge.exe");
        }
        Browser::Vivaldi => {
            add(&pf, r"Vivaldi\Application\vivaldi.exe");
            add(&pf86, r"Vivaldi\Application\vivaldi.exe");
            add(&local, r"Vivaldi\Application\vivaldi.exe");
        }
    }
    out
}

fn find_browser_bin(browser: Browser) -> Option<String> {
    let env_key = format!("KOPUZ_{}_BIN", browser.id().to_uppercase().replace('-', "_"));
    if let Some(v) = std::env::var_os(&env_key)
        && !v.is_empty()
    {
        return Some(v.to_string_lossy().into_owned());
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    for candidate in browser_candidates(browser) {
        for dir in &dirs {
            let p = dir.join(candidate);
            if p.is_file() {
                return Some(candidate.to_string());
            }
        }
    }
    #[cfg(target_os = "macos")]
    for path in macos_app_paths(browser) {
        if std::path::Path::new(path).is_file() {
            return Some((*path).to_string());
        }
    }
    #[cfg(target_os = "windows")]
    for path in windows_install_paths(browser) {
        if path.is_file() {
            return Some(path.to_string_lossy().into_owned());
        }
    }
    None
}

/// Wipe the isolated profile, launch the chosen browser at the Google
/// sign-in page, and poll the cookie SQLite until both SAPISID and SID
/// land. Returns the decrypted cookie header. The browser is always
/// killed before returning, success or timeout.
pub async fn launch_signin_and_extract(
    browser: Browser,
    server_id: &str,
    signin_timeout: Duration,
) -> Result<String, String> {
    let profile = profile_dir(server_id);
    match tokio::fs::remove_dir_all(&profile).await {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("wipe yt-profile: {e}")),
    }
    tokio::fs::create_dir_all(&profile)
        .await
        .map_err(|e| format!("mkdir yt-profile: {e}"))?;

    for name in ["SingletonLock", "SingletonCookie", "SingletonSocket"] {
        match tokio::fs::remove_file(profile.join(name)).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {}
        }
    }

    let bin = find_browser_bin(browser).ok_or_else(|| {
        format!(
            "{} not found in PATH (looked for: {}). Install it, or set ${}_BIN to its absolute path.",
            browser,
            browser_candidates(browser).join(", "),
            format!("KOPUZ_{}", browser.id().to_uppercase().replace('-', "_"))
        )
    })?;
    eprintln!(
        "[yt-signin] launching {bin} against {} (sign-in URL: {SIGNIN_URL})",
        profile.display()
    );
    let mut child = Command::new(&bin)
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg(SIGNIN_URL)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("spawn {bin}: {e}"))?;
    eprintln!("[yt-signin] {bin} pid={:?} — waiting for sign-in", child.id());

    let deadline = Instant::now() + signin_timeout;
    let mut last_extract_err: Option<String> = None;
    let outcome = loop {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if Instant::now() > deadline {
            let detail = last_extract_err
                .as_deref()
                .map(|e| format!("; last extract error: {e}"))
                .unwrap_or_default();
            break Err(format!(
                "Sign-in not detected within {}s — close the browser and try again{detail}",
                signin_timeout.as_secs()
            ));
        }
        if let Ok(Some(status)) = child.try_wait() {
            eprintln!("[yt-signin] {bin} exited early: {status}");
            break Err(format!(
                "Browser ({bin}) exited before sign-in completed (status {status}) — try again, or pick a different browser in settings"
            ));
        }
        let cookies = match super::cookies::extract_from(browser, &profile).await {
            Ok(c) => c,
            Err(e) => {
                if last_extract_err.as_deref() != Some(e.as_str()) {
                    eprintln!("[yt-signin] cookie extract: {e}");
                    last_extract_err = Some(e);
                }
                continue;
            }
        };
        if has_cookie(&cookies, "SAPISID") && has_cookie(&cookies, "SID") {
            eprintln!("[yt-signin] cookies detected — closing {bin}");
            break Ok(cookies);
        }
    };

    let _ = child.kill().await;
    outcome
}

fn has_cookie(header: &str, name: &str) -> bool {
    header
        .split(';')
        .any(|p| p.trim().split_once('=').is_some_and(|(k, _)| k == name))
}

pub fn delete_profile(server_id: &str) -> std::io::Result<()> {
    let path = profile_dir(server_id);
    match std::fs::remove_dir_all(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}
