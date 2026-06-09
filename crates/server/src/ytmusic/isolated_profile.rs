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
        .map(|d| {
            // Chromium profiles must live in Local AppData on Windows, not
            // Roaming (`config_dir()`): a Roaming profile that OneDrive syncs
            // locks the browser's files and it hangs loading every page.
            #[cfg(target_os = "windows")]
            let base = d.data_local_dir();
            #[cfg(not(target_os = "windows"))]
            let base = d.config_dir();
            base.join(&leaf)
        })
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

/// True when running inside a flatpak sandbox. The host browser binary isn't
/// reachable from the sandbox `/usr`, so launches are proxied to the host via
/// `flatpak-spawn --host` (which the runtime provides at `/usr/bin`).
fn in_flatpak() -> bool {
    std::path::Path::new("/.flatpak-info").exists()
}

/// Resolve the browser command on the *host* PATH (the sandbox can't stat host
/// binaries). Probes each candidate with `flatpak-spawn --host command -v`.
async fn find_host_browser_bin(browser: Browser) -> Option<String> {
    // Honour an explicit override (e.g. a non-standard host install path) — same
    // escape hatch as the native `find_browser_bin`. `flatpak-spawn --host` runs
    // it in the host environment, so an absolute host path works.
    let env_key = format!("KOPUZ_{}_BIN", browser.id().to_uppercase().replace('-', "_"));
    if let Some(v) = std::env::var_os(&env_key)
        && !v.is_empty()
    {
        return Some(v.to_string_lossy().into_owned());
    }
    for cand in browser_candidates(browser) {
        let ok = Command::new("flatpak-spawn")
            .args(["--host", "sh", "-c"])
            .arg(format!("command -v {cand}"))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            return Some(cand.to_string());
        }
    }
    None
}

/// The browser launch command — transparently escapes the flatpak sandbox via
/// `flatpak-spawn --host` when packaged, a plain `Command` natively.
/// `--watch-bus` ties the host browser's lifetime to ours, so `child.kill()`
/// (and `kill_on_drop`) still tears it down.
fn browser_command(bin: &str) -> Command {
    if in_flatpak() {
        let mut c = Command::new("flatpak-spawn");
        c.args(["--host", "--watch-bus", bin]);
        c
    } else {
        Command::new(bin)
    }
}

/// Wipe the isolated profile, launch the chosen browser at the Google
/// sign-in page, and poll the cookie SQLite until both SAPISID and SID
/// land. Returns the decrypted cookie header. The browser is always
/// killed before returning, success or timeout.
///
// TODO(windows-signin): browser sign-in is disabled on Windows in the
// UI (settings_popups.rs forces anonymous mode there) because the
// Google accounts page renders a blank document inside the isolated
// `--user-data-dir` profile — the omnibox shows the ServiceLogin URL
// but the page body is about:blank, so SAPISID/SID never land and
// this loops to timeout. Linux/macOS work. Likely Edge/Chrome
// first-run + automation heuristics specific to Windows; needs a
// Windows tester to iterate (tried --disable-blink-features=
// AutomationControlled + UA spoof, reverted — see commits
// 6bec69d/8a03c89). Until then, Windows users get anonymous YT.
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

    let bin = if in_flatpak() {
        find_host_browser_bin(browser).await.ok_or_else(|| {
            format!(
                "{} not found on the host (looked for: {}). Install it on the host system.",
                browser,
                browser_candidates(browser).join(", ")
            )
        })?
    } else {
        find_browser_bin(browser).ok_or_else(|| {
            format!(
                "{} not found in PATH (looked for: {}). Install it, or set $KOPUZ_{}_BIN to its absolute path.",
                browser,
                browser_candidates(browser).join(", "),
                browser.id().to_uppercase().replace('-', "_")
            )
        })?
    };
    // Windows: disable App-Bound Encryption for this browser (official policy)
    // so the isolated profile writes v11 (DPAPI) cookies we can decrypt — v20
    // ABE cookies are unreadable by external apps by design. The guard restores
    // the previous policy value on drop, even on error/timeout.
    #[cfg(target_os = "windows")]
    let _abe = AbeDisabled::disable(browser);

    eprintln!(
        "[yt-signin] launching {bin} against {} (sign-in URL: {SIGNIN_URL})",
        profile.display()
    );
    let mut cmd = browser_command(&bin);
    cmd.arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg(format!("--user-data-dir={}", profile.display()));
    // kopuz's own UI is WebView2 (Chromium), so kopuz runs inside a Windows job
    // object. A spawned Chrome inherits it, and Chromium's sandbox can't create
    // the nested jobs its renderer/GPU children need (the sandbox job's
    // active-process quota is 1) — the window opens but the content is dead (no
    // DOM, no network, not even right-clickable). CREATE_BREAKAWAY_FROM_JOB
    // (0x01000000) detaches the child from kopuz's job so the browser's own
    // sandbox jobs can be created.
    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x0100_0000);
    let mut child = cmd
        .arg(SIGNIN_URL)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("spawn {bin}: {e}"))?;
    eprintln!("[yt-signin] {bin} pid={:?} — waiting for sign-in", child.id());

    let deadline = Instant::now() + signin_timeout;
    let mut last_extract_err: Option<String> = None;
    // Edge / Chrome on Windows often spawn the visible UI in detached
    // processes and the launched parent exits with status 0 in <1s.
    // Polling the cookie SQLite still works — it's on disk in the
    // profile dir we control. Track child exit but DON'T bail on it;
    // wait for cookies up to the full timeout. If a non-zero exit
    // happens (crash) we still tolerate it for the same reason.
    let mut child_exited_at: Option<Instant> = None;
    let outcome = loop {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if Instant::now() > deadline {
            let detail = last_extract_err
                .as_deref()
                .map(|e| format!("; last extract error: {e}"))
                .unwrap_or_default();
            let exited_note = child_exited_at
                .map(|_| " — note: the browser process exited early (likely detached UI); close all browser windows and try again")
                .unwrap_or_default();
            break Err(format!(
                "Sign-in not detected within {}s{exited_note}{detail}",
                signin_timeout.as_secs()
            ));
        }
        if child_exited_at.is_none()
            && let Ok(Some(status)) = child.try_wait()
        {
            eprintln!("[yt-signin] {bin} exited (status {status}) — continuing to poll cookies in case the browser is still running as a detached process");
            child_exited_at = Some(Instant::now());
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

/// HKCU policy subkey for a browser's `ApplicationBoundEncryptionEnabled`.
#[cfg(target_os = "windows")]
fn abe_policy_subkey(browser: Browser) -> &'static str {
    match browser {
        Browser::Chrome => r"SOFTWARE\Policies\Google\Chrome",
        Browser::Edge => r"SOFTWARE\Policies\Microsoft\Edge",
        Browser::Brave => r"SOFTWARE\Policies\BraveSoftware\Brave",
        Browser::Chromium => r"SOFTWARE\Policies\Chromium",
        Browser::Vivaldi => r"SOFTWARE\Policies\Vivaldi\Vivaldi",
    }
}

/// Sets `ApplicationBoundEncryptionEnabled=0` (HKCU policy) so a freshly
/// launched browser writes v11/DPAPI cookies instead of v20/ABE — which an
/// external app can decrypt. Restores the prior value (or removes it) on drop.
#[cfg(target_os = "windows")]
struct AbeDisabled {
    subkey: &'static str,
    prev: Option<u32>,
}

#[cfg(target_os = "windows")]
impl AbeDisabled {
    const VALUE: &'static str = "ApplicationBoundEncryptionEnabled";

    fn disable(browser: Browser) -> Option<Self> {
        use winreg::RegKey;
        use winreg::enums::HKEY_CURRENT_USER;
        let subkey = abe_policy_subkey(browser);
        let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey(subkey)
            .ok()?;
        let prev: Option<u32> = key.get_value(Self::VALUE).ok();
        key.set_value(Self::VALUE, &0u32).ok()?;
        eprintln!("[yt-signin] disabled App-Bound Encryption policy for sign-in (restored after)");
        Some(Self { subkey, prev })
    }
}

#[cfg(target_os = "windows")]
impl Drop for AbeDisabled {
    fn drop(&mut self) {
        use winreg::RegKey;
        use winreg::enums::{HKEY_CURRENT_USER, KEY_SET_VALUE};
        if let Ok(key) =
            RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(self.subkey, KEY_SET_VALUE)
        {
            let _ = match self.prev {
                Some(v) => key.set_value(Self::VALUE, &v),
                None => key.delete_value(Self::VALUE),
            };
        }
    }
}

pub fn delete_profile(server_id: &str) -> std::io::Result<()> {
    let path = profile_dir(server_id);
    match std::fs::remove_dir_all(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}
