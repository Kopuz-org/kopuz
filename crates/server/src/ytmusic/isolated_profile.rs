//! Manages kopuz's isolated browser profile at
//! ~/.config/kopuz/yt-profile/, used only for the one-time YouTube
//! Music sign-in. The user's real browser profile is never touched.
//!
//! Flow: wipe → spawn `<browser> --user-data-dir=<isolated>
//! https://music.youtube.com/` so the user can sign in interactively →
//! poll the isolated profile's cookie SQLite until the auth cookies
//! appear (SAPISID + SID) → kill the browser → return the decrypted
//! cookie jar for kopuz to store. From there
//! [`super::verify_session_keepalive`] keeps the session alive over
//! HTTP without ever touching the browser again.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use config::Browser;

pub fn profile_dir() -> PathBuf {
    directories::ProjectDirs::from("com", "temidaradev", "kopuz")
        .map(|d| d.config_dir().join("yt-profile"))
        .unwrap_or_else(|| PathBuf::from("./yt-profile"))
}

fn browser_binary(browser: Browser) -> &'static str {
    match browser {
        Browser::Brave => "brave",
        Browser::Chrome => "google-chrome",
        Browser::Chromium => "chromium",
        Browser::Edge => "microsoft-edge",
        Browser::Vivaldi => "vivaldi",
    }
}

/// Wipe the isolated profile, launch the chosen browser pointed at
/// music.youtube.com so the user can sign in, then poll the cookie
/// SQLite until the auth cookies appear. Once both SAPISID and SID are
/// present the browser is killed and the decrypted cookie header is
/// returned. Times out after `signin_timeout`.
pub async fn launch_signin_and_extract(
    browser: Browser,
    signin_timeout: Duration,
) -> Result<String, String> {
    let profile = profile_dir();
    if profile.exists() {
        std::fs::remove_dir_all(&profile)
            .map_err(|e| format!("wipe yt-profile: {e}"))?;
    }
    std::fs::create_dir_all(&profile)
        .map_err(|e| format!("mkdir yt-profile: {e}"))?;

    let bin = browser_binary(browser);
    let child = Command::new(bin)
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("https://accounts.google.com/ServiceLogin?service=youtube&continue=https%3A%2F%2Fmusic.youtube.com%2F")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn {bin}: {e}"))?;
    let pid = child.id();

    let deadline = Instant::now() + signin_timeout;
    let cookies = loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        if Instant::now() > deadline {
            kill_pid(pid);
            return Err(format!(
                "Sign-in not detected within {}s — close the browser and try again",
                signin_timeout.as_secs()
            ));
        }
        let Ok(cookies) = super::cookies::extract_from(browser, &profile).await else {
            continue;
        };
        if cookies.contains("SAPISID=") && cookies.contains("SID=") {
            break cookies;
        }
    };

    kill_pid(pid);
    tokio::time::sleep(Duration::from_millis(500)).await;
    Ok(cookies)
}

fn kill_pid(pid: u32) {
    let _ = Command::new("kill").args(["-TERM", &pid.to_string()]).status();
    std::thread::sleep(Duration::from_millis(1500));
    let _ = Command::new("kill").args(["-KILL", &pid.to_string()]).status();
}
