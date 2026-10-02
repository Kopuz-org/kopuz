use std::path::{Path, PathBuf};

use config::{Browser, BrowserEngine};
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
        Browser::Brave => &["brave", "brave-browser", "brave-origin"],
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
        Browser::Firefox => &["firefox", "firefox-esr", "firefox-bin"],
        Browser::LibreWolf => &["librewolf"],
        Browser::Zen => &["zen-browser", "zen"],
        Browser::Floorp => &["floorp"],
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
        Browser::Firefox => &["org.mozilla.firefox"],
        Browser::LibreWolf => &["io.gitlab.librewolf-community"],
        Browser::Zen => &["app.zen_browser.zen"],
        Browser::Floorp => &["one.ablaze.floorp"],
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
        Browser::Firefox => &["/Applications/Firefox.app/Contents/MacOS/firefox"],
        Browser::LibreWolf => &["/Applications/LibreWolf.app/Contents/MacOS/librewolf"],
        Browser::Zen => &[
            "/Applications/Zen.app/Contents/MacOS/zen",
            "/Applications/Zen Browser.app/Contents/MacOS/zen",
        ],
        Browser::Floorp => &["/Applications/Floorp.app/Contents/MacOS/floorp"],
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
        Browser::Firefox => {
            add(&pf, r"Mozilla Firefox\firefox.exe");
            add(&pf86, r"Mozilla Firefox\firefox.exe");
            add(&local, r"Mozilla Firefox\firefox.exe");
        }
        Browser::LibreWolf => {
            add(&pf, r"LibreWolf\librewolf.exe");
            add(&pf86, r"LibreWolf\librewolf.exe");
            add(&local, r"LibreWolf\librewolf.exe");
        }
        Browser::Zen => {
            add(&pf, r"Zen Browser\zen.exe");
            add(&pf86, r"Zen Browser\zen.exe");
            add(&local, r"Zen Browser\zen.exe");
            add(&local, r"Zen\zen.exe");
        }
        Browser::Floorp => {
            add(&pf, r"Floorp\floorp.exe");
            add(&pf86, r"Floorp\floorp.exe");
            add(&local, r"Floorp\floorp.exe");
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

/// Resolve a browser to something spawnable. `profile` is the isolated
/// user-data dir a cookie import needs exported into the flatpak run; a
/// caller that just wants the user's own browser opened passes `None`.
pub(crate) async fn find_browser_bin(
    browser: Browser,
    profile: Option<&std::path::Path>,
) -> Option<BrowserBin> {
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

    let export = profile
        .map(|p| format!(" --filesystem={}", p.display()))
        .unwrap_or_default();

    if let Ok(v) = std::env::var("KOPUZ_BROWSER_FLATPAK_ID")
        && !v.trim().is_empty()
    {
        let id = v.to_string().to_owned();
        if check_browser_command(format!("flatpak info {id}")).await {
            return Some(BrowserBin::CommandLine(format!("flatpak run{export} {id}")));
        }
    }

    for cand in browser_flatpak_ids(browser) {
        if check_browser_command(format!("flatpak info {cand}")).await {
            return Some(BrowserBin::CommandLine(format!(
                "flatpak run{export} {cand}"
            )));
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
/// so `child.kill()`/`kill_on_drop` still tears the host browser down: a
/// cookie import owns the browser it spawned and wants it gone when kopuz
/// drops the child.
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
        c.arg("--host");
        c.arg("--watch-bus");
        c.args(&tokens);
        c
    } else {
        let mut c = Command::new(tokens[0]);
        c.args(&tokens[1..]);
        c
    }
}

/// Seed the freshly wiped profile with whatever the engine needs before its
/// first run.
pub(crate) fn prepare_profile(browser: Browser, profile: &Path) {
    match browser.engine() {
        BrowserEngine::Chromium => prepare_chromium_profile(browser, profile),
        BrowserEngine::Gecko => prepare_gecko_profile(profile),
    }
}

/// Windows: seed a NONE-protected app-bound key into the fresh profile before
/// launch, so v20 cookies stay decryptable if Google's Finch-gated App-Bound
/// rollout flips on. Best-effort — today's cookies are v10 (DPAPI). No-op
/// elsewhere.
#[cfg(target_os = "windows")]
fn prepare_chromium_profile(browser: Browser, profile: &Path) {
    if let Err(e) = super::windows_native::plant_app_bound_key(browser, profile) {
        tracing::warn!(error = %e, "app-bound key plant failed — v20 cookies (if any) won't decrypt; v10 still works");
    }
}

#[cfg(not(target_os = "windows"))]
fn prepare_chromium_profile(_browser: Browser, _profile: &Path) {}

/// Gecko has no `--app` window and no "skip the tour" flag, so a fresh profile
/// otherwise opens onboarding tabs over the sign-in page. `user.js` is read at
/// every startup, before anything is drawn.
fn prepare_gecko_profile(profile: &Path) {
    const PREFS: &str = concat!(
        "user_pref(\"browser.aboutwelcome.enabled\", false);\n",
        "user_pref(\"browser.startup.homepage_override.mstone\", \"ignore\");\n",
        "user_pref(\"browser.shell.checkDefaultBrowser\", false);\n",
        "user_pref(\"datareporting.policy.dataSubmissionEnabled\", false);\n",
        "user_pref(\"browser.sessionstore.resume_from_crash\", false);\n",
    );
    if let Err(e) = std::fs::write(profile.join("user.js"), PREFS) {
        tracing::warn!(error = %e, "could not seed user.js — the sign-in window may open onboarding tabs");
    }
}

/// The command that opens `url` in a throwaway profile under `profile`. The
/// two engines share nothing here: Chromium takes one `--user-data-dir` and
/// can open a chromeless `--app` window, Gecko takes `--profile` and needs
/// `--no-remote` or it hands the URL to the user's own running browser — and
/// signs them in there, outside the profile kopuz reads back.
pub(crate) fn signin_command(
    browser: Browser,
    bin: &BrowserBin,
    profile: &Path,
    url: &str,
) -> Command {
    let mut cmd = browser_command(bin);
    match browser.engine() {
        BrowserEngine::Chromium => {
            cmd.arg("--no-first-run")
                .arg("--no-default-browser-check")
                .arg("--password-store=basic")
                .arg(format!("--user-data-dir={}", profile.display()))
                .arg(format!("--app={url}"));
        }
        BrowserEngine::Gecko => {
            cmd.arg("--no-remote")
                .arg("--profile")
                .arg(profile)
                .arg("--new-window")
                .arg(url);
        }
    }
    // Windows: kopuz's WebView2 UI runs us inside a job object whose sandbox
    // quota (1 active process) stops a spawned browser from creating the nested
    // jobs its renderer/GPU need — the window opens but the content is dead.
    // CREATE_BREAKAWAY_FROM_JOB detaches the child so its own sandbox works.
    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x0100_0000);
    cmd.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    cmd
}

/// Run a command and hand back its stdout, through the host when sandboxed.
async fn command_stdout(program: &str, args: &[&str]) -> Option<String> {
    let mut command = if in_flatpak() {
        let mut c = Command::new("flatpak-spawn");
        c.arg("--host").arg(program);
        c
    } else {
        Command::new(program)
    };
    let out = command
        .args(args)
        .stderr(std::process::Stdio::null())
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Map whatever the OS calls its default handler — a desktop file id, a bundle
/// id, a Windows ProgId — onto a browser kopuz can drive. Matched on substrings
/// because each platform spells the same browser differently, most specific
/// first so `chromium` is never read as Chrome.
fn browser_from_handler(handler: &str) -> Option<Browser> {
    const KEYS: &[(&str, Browser)] = &[
        ("librewolf", Browser::LibreWolf),
        ("floorp", Browser::Floorp),
        ("zen", Browser::Zen),
        ("firefox", Browser::Firefox),
        ("chromium", Browser::Chromium),
        ("chrome", Browser::Chrome),
        ("brave", Browser::Brave),
        ("edge", Browser::Edge),
        ("vivaldi", Browser::Vivaldi),
        ("helium", Browser::Helium),
    ];
    let handler = handler.to_ascii_lowercase();
    KEYS.iter()
        .find(|(key, _)| handler.contains(key))
        .map(|(_, browser)| *browser)
}

#[cfg(all(unix, not(target_os = "macos")))]
async fn default_handler() -> Option<String> {
    if let Some(out) = command_stdout("xdg-settings", &["get", "default-web-browser"]).await
        && !out.trim().is_empty()
    {
        return Some(out);
    }
    command_stdout("xdg-mime", &["query", "default", "x-scheme-handler/https"]).await
}

/// macOS keeps the URL handlers in a binary plist, so read it through `plutil`
/// rather than linking LaunchServices.
#[cfg(target_os = "macos")]
async fn default_handler() -> Option<String> {
    let home = std::env::var_os("HOME")?;
    let plist = PathBuf::from(home)
        .join("Library/Preferences/com.apple.LaunchServices/com.apple.launchservices.secure.plist");
    let json = command_stdout(
        "plutil",
        &["-convert", "json", "-o", "-", &plist.to_string_lossy()],
    )
    .await?;
    let parsed: serde_json::Value = serde_json::from_str(&json).ok()?;
    parsed
        .get("LSHandlers")?
        .as_array()?
        .iter()
        .find(|handler| handler.get("LSHandlerURLScheme").and_then(|s| s.as_str()) == Some("https"))
        .and_then(|handler| {
            handler
                .get("LSHandlerRoleAll")
                .or_else(|| handler.get("LSHandlerRoleViewer"))
        })
        .and_then(|role| role.as_str())
        .map(str::to_owned)
}

#[cfg(target_os = "windows")]
async fn default_handler() -> Option<String> {
    let out = command_stdout(
        "reg",
        &[
            "query",
            r"HKCU\SOFTWARE\Microsoft\Windows\Shell\Associations\UrlAssociations\https\UserChoice",
            "/v",
            "ProgId",
        ],
    )
    .await?;
    out.lines()
        .find(|line| line.contains("ProgId"))
        .and_then(|line| line.split_whitespace().next_back())
        .map(str::to_owned)
}

/// The system's default browser, when it is one kopuz knows how to drive.
pub async fn detect_default_browser() -> Option<Browser> {
    let handler = default_handler().await?;
    let browser = browser_from_handler(handler.trim());
    tracing::debug!(handler = handler.trim(), resolved = ?browser.map(Browser::id), "read the system default browser");
    browser
}

async fn is_installed(browser: Browser) -> bool {
    find_browser_bin(browser, None).await.is_some()
}

/// Which browser a sign-in actually opens. A stored choice is honoured as-is;
/// without one kopuz follows the system default, which is the only answer that
/// is right for a Firefox user without them having to say so. The scan is the
/// last resort — and its first hit, not Chrome, because an unconfigured
/// machine should still sign in rather than report Chrome missing.
pub async fn resolve_browser(preferred: Option<Browser>) -> Browser {
    if let Some(browser) = preferred {
        return browser;
    }
    let detected = detect_default_browser().await;
    if let Some(browser) = detected
        && is_installed(browser).await
    {
        tracing::info!(
            browser = browser.id(),
            "signing in with the system default browser"
        );
        return browser;
    }
    for browser in Browser::ALL.iter().copied() {
        if is_installed(browser).await {
            tracing::info!(
                browser = browser.id(),
                "no usable system default browser; using the first one installed"
            );
            return browser;
        }
    }
    // Nothing is installed. Name the default anyway, so the launch failure
    // names the browser the user actually set.
    detected.unwrap_or(Browser::Firefox)
}

pub async fn has_host_spawn() -> bool {
    if !in_flatpak() {
        return true;
    }

    Command::new("flatpak-spawn")
        .args(["--host", "true"])
        .status()
        .await
        .is_ok_and(|status| status.success())
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

    fn signin_args(browser: Browser) -> Vec<String> {
        let bin = BrowserBin::Path("browser".to_string());
        signin_command(
            browser,
            &bin,
            std::path::Path::new("/tmp/kopuz-profile"),
            "https://example.test/signin",
        )
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
    }

    #[test]
    fn a_gecko_sign_in_is_isolated_from_the_running_browser() {
        let args = signin_args(Browser::Firefox);
        // Without --no-remote the URL is handed to the user's own Firefox,
        // which signs them in outside the profile kopuz reads back.
        assert!(args.contains(&"--no-remote".to_string()));
        assert!(args.contains(&"--profile".to_string()));
        assert!(args.contains(&"/tmp/kopuz-profile".to_string()));
        assert!(args.contains(&"https://example.test/signin".to_string()));
        assert!(!args.iter().any(|a| a.starts_with("--user-data-dir")));
    }

    #[test]
    fn a_chromium_sign_in_keeps_its_own_profile_flags() {
        let args = signin_args(Browser::Chrome);
        assert!(args.contains(&"--user-data-dir=/tmp/kopuz-profile".to_string()));
        assert!(args.contains(&"--app=https://example.test/signin".to_string()));
        assert!(!args.contains(&"--no-remote".to_string()));
    }

    #[test]
    fn a_default_handler_names_the_browser_it_belongs_to() {
        // Desktop ids, bundle ids and Windows ProgIds for the same browser.
        for handler in ["firefox.desktop", "org.mozilla.firefox", "FirefoxURL"] {
            assert_eq!(browser_from_handler(handler), Some(Browser::Firefox));
        }
        assert_eq!(
            browser_from_handler("io.gitlab.librewolf-community.desktop"),
            Some(Browser::LibreWolf)
        );
        // "chromium" must not read as Chrome.
        assert_eq!(
            browser_from_handler("org.chromium.Chromium"),
            Some(Browser::Chromium)
        );
        assert_eq!(
            browser_from_handler("google-chrome.desktop"),
            Some(Browser::Chrome)
        );
        assert_eq!(browser_from_handler("MSEdgeHTM"), Some(Browser::Edge));
        assert_eq!(browser_from_handler("com.apple.safari"), None);
    }
}
