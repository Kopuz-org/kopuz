use std::path::PathBuf;

use config::Browser;
use tokio::process::Command;

/// How to invoke the browser. The two shapes must not be conflated: a `Path`
/// may contain spaces (macOS app bundles, Windows Program Files) and is never
/// split, while a `CommandLine` is multi-token by construction (`flatpak run
/// <id>`, `$KOPUZ_BROWSER_COMMAND`) and is split on whitespace. Guessing the
/// shape from the string is exactly what broke spawning `/Applications/Google
/// Chrome.app/...` (#513) and `C:\Program Files\...` before it (#435).
#[derive(Debug, Clone)]
pub(crate) enum BrowserBin {
    Path(String),
    CommandLine(String),
}

impl std::fmt::Display for BrowserBin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (Self::Path(s) | Self::CommandLine(s)) = self;
        f.write_str(s)
    }
}

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
        Browser::Helium => &["helium-browser", "helium"],
    }
}

pub(crate) fn browser_flatpak_ids(browser: Browser) -> &'static [&'static str] {
    match browser {
        Browser::Brave => &["com.brave.Browser"],
        Browser::Chrome => &["com.google.Chrome", "com.google.ChromeDev"],
        Browser::Chromium => &["org.chromium.Chromium"],
        Browser::Edge => &["com.microsoft.Edge"],
        Browser::Vivaldi => &["com.vivaldi.Vivaldi"],
        // Helium ships .deb/AppImage/tarball upstream, no flatpak.
        Browser::Helium => &[],
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
        Browser::Helium => &["/Applications/Helium.app/Contents/MacOS/Helium"],
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
        Browser::Helium => {
            add(&pf, r"imput\Helium\Application\chrome.exe");
            add(&pf86, r"imput\Helium\Application\chrome.exe");
            add(&local, r"imput\Helium\Application\chrome.exe");
        }
    }
    out
}

/// True inside a flatpak sandbox, where the host browser is only reachable via
/// `flatpak-spawn --host`.
pub(crate) fn in_flatpak() -> bool {
    std::path::Path::new("/.flatpak-info").exists()
}

/// True if the command does not error, uses `sh -c` for executing in shell
/// If running in flatpak container uses `flatpak-spawn --host`.
pub(crate) async fn check_browser_command(arg: String) -> bool {
    let mut command = if in_flatpak() {
        let mut c = Command::new("flatpak-spawn");
        c.args(["--host", "sh", "-c"]);
        c
    } else {
        let mut c = Command::new("sh");
        c.arg("-c");
        c
    };

    command.arg(arg);

    command
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

pub(crate) async fn find_browser_bin(browser: Browser) -> Option<BrowserBin> {
    let env_key = format!(
        "KOPUZ_{}_BIN",
        browser.id().to_uppercase().replace('-', "_")
    );
    if let Some(v) = std::env::var_os(&env_key)
        && !v.is_empty()
    {
        return Some(BrowserBin::Path(v.to_string_lossy().into_owned()));
    }

    if in_flatpak() {
        for cand in browser_candidates(browser) {
            if check_browser_command(format!("command -v {cand}")).await {
                return Some(BrowserBin::Path(cand.to_string()));
            }
        }
    } else {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
        for candidate in browser_candidates(browser) {
            for dir in &dirs {
                let p = dir.join(candidate);
                if p.is_file() {
                    return Some(BrowserBin::Path(candidate.to_string()));
                }
            }
        }
    }

    if let Ok(v) = std::env::var("KOPUZ_BROWSER_FLATPAK_ID")
        && !v.trim().is_empty()
    {
        let id = v.to_string().to_owned();
        if check_browser_command(format!("flatpak info {id}")).await {
            return Some(BrowserBin::CommandLine(format!("flatpak run {id}")));
        }
    }

    for cand in browser_flatpak_ids(browser) {
        if check_browser_command(format!("flatpak info {cand}")).await {
            return Some(BrowserBin::CommandLine(format!("flatpak run {cand}")));
        }
    }

    #[cfg(target_os = "macos")]
    for path in macos_app_paths(browser) {
        if std::path::Path::new(path).is_file() {
            return Some(BrowserBin::Path((*path).to_string()));
        }
    }
    #[cfg(target_os = "windows")]
    for path in windows_install_paths(browser) {
        if path.is_file() {
            return Some(BrowserBin::Path(path.to_string_lossy().into_owned()));
        }
    }
    None
}

/// Plain `Command` natively; `flatpak-spawn --host --watch-bus` when packaged,
/// so `child.kill()`/`kill_on_drop` still tears the host browser down.
pub(crate) fn browser_command(bin: &BrowserBin) -> Command {
    let tokens: Vec<&str> = match bin {
        BrowserBin::Path(p) => vec![p.as_str()],
        BrowserBin::CommandLine(c) => {
            let split: Vec<&str> = c.split_whitespace().collect();
            if split.is_empty() {
                vec![c.as_str()]
            } else {
                split
            }
        }
    };
    if in_flatpak() {
        let mut c = Command::new("flatpak-spawn");
        c.args(["--host", "--watch-bus"]);
        c.args(&tokens);
        c
    } else {
        let mut c = Command::new(tokens[0]);
        c.args(&tokens[1..]);
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_with_spaces_is_never_split() {
        // The #513 shape: a macOS app-bundle binary. Splitting it spawned
        // "/Applications/Google" and failed with ENOENT.
        let bin = BrowserBin::Path(
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".to_string(),
        );
        let cmd = browser_command(&bin);
        assert_eq!(
            cmd.as_std().get_program().to_string_lossy(),
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
        );
        assert_eq!(cmd.as_std().get_args().count(), 0);
    }

    #[test]
    fn a_command_line_is_split_into_tokens() {
        let bin = BrowserBin::CommandLine("flatpak run com.google.Chrome".to_string());
        let cmd = browser_command(&bin);
        assert_eq!(cmd.as_std().get_program().to_string_lossy(), "flatpak");
        let args: Vec<String> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, ["run", "com.google.Chrome"]);
    }
}
