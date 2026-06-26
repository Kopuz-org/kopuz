use std::path::PathBuf;

use config::Browser;
use tokio::process::Command;

pub(crate) fn browser_candidates(browser: Browser) -> &'static [&'static str] {
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
        Browser::Brave => &["/Applications/Brave Browser.app/Contents/MacOS/Brave Browser"],
        Browser::Chrome => &["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"],
        Browser::Chromium => &["/Applications/Chromium.app/Contents/MacOS/Chromium"],
        Browser::Edge => &["/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"],
        Browser::Vivaldi => &["/Applications/Vivaldi.app/Contents/MacOS/Vivaldi"],
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

pub(crate) fn find_browser_bin(browser: Browser) -> Option<String> {
    let env_key = format!(
        "KOPUZ_{}_BIN",
        browser.id().to_uppercase().replace('-', "_")
    );
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

/// True inside a flatpak sandbox, where the host browser is only reachable via
/// `flatpak-spawn --host`.
pub(crate) fn in_flatpak() -> bool {
    std::path::Path::new("/.flatpak-info").exists()
}

/// Resolve the browser on the *host* PATH (the sandbox can't stat host
/// binaries), probing each candidate with `flatpak-spawn --host command -v`.
pub(crate) async fn find_host_browser_bin(browser: Browser) -> Option<String> {
    let env_key = format!(
        "KOPUZ_{}_BIN",
        browser.id().to_uppercase().replace('-', "_")
    );
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

/// Plain `Command` natively; `flatpak-spawn --host --watch-bus` when packaged,
/// so `child.kill()`/`kill_on_drop` still tears the host browser down.
pub(crate) fn browser_command(bin: &str) -> Command {
    if in_flatpak() {
        let mut c = Command::new("flatpak-spawn");
        c.args(["--host", "--watch-bus", bin]);
        c
    } else {
        Command::new(bin)
    }
}
