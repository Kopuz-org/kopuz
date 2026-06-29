use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use config::Browser;
use tokio::process::Child;

use super::browser::{
    browser_candidates, browser_command, find_browser_bin, in_flatpak,
};
use super::profile::profile_dir;

/// Wipe the `<prefix>-<server_id>` profile, launch `browser` at `signin_url`,
/// then wait via `extract` until it yields the signed-in result. `extract`
/// returns `Ok(Some(value))` when done, `Ok(None)` while pending, `Err` for a
/// transient read error (logged, retried). The browser is always killed before
/// returning. The wait strategy is platform-specific (see `wait_for_signin`).
pub async fn launch_signin_and_extract<F, Fut>(
    browser: Browser,
    server_id: &str,
    prefix: &str,
    signin_url: &str,
    signin_timeout: Duration,
    extract: F,
) -> Result<String, String>
where
    F: Fn(Browser, PathBuf) -> Fut,
    Fut: Future<Output = Result<Option<String>, String>>,
{
    let profile = profile_dir(prefix, server_id);
    tracing::debug!(prefix, url = signin_url, profile = %profile.display(), timeout_s = signin_timeout.as_secs(), "preparing isolated sign-in profile");
    match tokio::fs::remove_dir_all(&profile).await {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("wipe {prefix}: {e}")),
    }
    tokio::fs::create_dir_all(&profile)
        .await
        .map_err(|e| format!("mkdir {prefix}: {e}"))?;

    for name in ["SingletonLock", "SingletonCookie", "SingletonSocket"] {
        let _ = tokio::fs::remove_file(profile.join(name)).await;
    }

    prepare_profile(browser, &profile);
    

    let bin = if let Ok(v) = std::env::var("KOPUZ_BROWSER_COMMAND") {
            tracing::info!("$KOPUZ_BROWSER_COMMAND used to override browser command");
            v.to_string().to_owned()
    } else {
        let error = if in_flatpak() {
                format!(
                    "{browser} not found on the host (looked for: {}). Install it on the host system, or set $KOPUZ_{}_BIN.",
                    browser_candidates(browser).join(", "),
                    browser.id().to_uppercase().replace('-', "_")
                )
            } else {
                format!(
                    "{browser} not found in PATH (looked for: {}). Install it, or set $KOPUZ_{}_BIN.",
                    browser_candidates(browser).join(", "),
                    browser.id().to_uppercase().replace('-', "_")
                )
            };
        find_browser_bin(browser).await.ok_or_else(|| {error})?
    };
    tracing::info!(%bin, profile = %profile.display(), "launching sign-in browser");
    let mut cmd = browser_command(&bin);
    cmd.arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg(format!("--user-data-dir={}", profile.display()));
    // Windows: kopuz's WebView2 UI runs us inside a job object whose sandbox
    // quota (1 active process) stops a spawned Chrome from creating the nested
    // jobs its renderer/GPU need — the window opens but the content is dead.
    // CREATE_BREAKAWAY_FROM_JOB detaches the child so its own sandbox works.
    #[cfg(target_os = "windows")]
    {
        // tokio's Command has an inherent `creation_flags` on Windows.
        cmd.creation_flags(0x0100_0000);
    }
    let mut child = cmd
        .arg(signin_url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("spawn {bin}: {e}"))?;
    tracing::debug!(%bin, pid = ?child.id(), "browser spawned — waiting for sign-in");

    let wait = SigninWait {
        browser,
        profile: &profile,
        bin: &bin,
        timeout: signin_timeout,
    };
    let outcome = wait_for_signin(&wait, &mut child, &extract).await;

    let _ = child.kill().await;
    outcome
}

/// Fixed inputs for a sign-in wait, shared by both platform strategies.
struct SigninWait<'a> {
    browser: Browser,
    profile: &'a Path,
    bin: &'a str,
    timeout: Duration,
}

/// Windows: seed a NONE-protected app-bound key into the fresh profile before
/// launch, so v20 cookies stay decryptable if Google's Finch-gated App-Bound
/// rollout flips on. Best-effort — today's cookies are v10 (DPAPI). No-op
/// elsewhere.
#[cfg(target_os = "windows")]
fn prepare_profile(browser: Browser, profile: &Path) {
    if let Err(e) = super::windows_native::plant_app_bound_key(browser, profile) {
        tracing::warn!(error = %e, "app-bound key plant failed — v20 cookies (if any) won't decrypt; v10 still works");
    }
}

#[cfg(not(target_os = "windows"))]
fn prepare_profile(_browser: Browser, _profile: &Path) {}

/// Non-Windows: Chrome commits cookies promptly, so poll `extract` every 500ms —
/// the read succeeds while the sign-in window is still open.
#[cfg(not(target_os = "windows"))]
async fn wait_for_signin<F, Fut>(
    w: &SigninWait<'_>,
    child: &mut Child,
    extract: &F,
) -> Result<String, String>
where
    F: Fn(Browser, PathBuf) -> Fut,
    Fut: Future<Output = Result<Option<String>, String>>,
{
    let started = Instant::now();
    let deadline = started + w.timeout;
    let mut last_extract_err: Option<String> = None;
    // Edge/Chrome sometimes spawn the UI detached and the launcher exits early;
    // the store is still on disk, so keep polling regardless of exit.
    let mut child_exited_at: Option<Instant> = None;
    loop {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if Instant::now() > deadline {
            let detail = last_extract_err
                .as_deref()
                .map(|e| format!("; last extract error: {e}"))
                .unwrap_or_default();
            let exited_note = child_exited_at
                .map(|_| " — note: the browser exited early (likely detached UI); close all browser windows and try again")
                .unwrap_or_default();
            tracing::warn!(
                bin = w.bin,
                timeout_s = w.timeout.as_secs(),
                exited_early = child_exited_at.is_some(),
                "sign-in timed out"
            );
            return Err(format!(
                "Sign-in not detected within {}s{exited_note}{detail}",
                w.timeout.as_secs()
            ));
        }
        if child_exited_at.is_none()
            && let Ok(Some(status)) = child.try_wait()
        {
            tracing::debug!(bin = w.bin, %status, "browser exited — still polling cookies");
            child_exited_at = Some(Instant::now());
        }
        match extract(w.browser, w.profile.to_path_buf()).await {
            Ok(Some(value)) => {
                tracing::info!(
                    bin = w.bin,
                    elapsed_ms = started.elapsed().as_millis(),
                    "sign-in detected"
                );
                return Ok(value);
            }
            Ok(None) => {}
            Err(e) => {
                if last_extract_err.as_deref() != Some(e.as_str()) {
                    tracing::trace!(error = %e, "cookie extract not ready yet");
                    last_extract_err = Some(e);
                }
            }
        }
    }
}

/// Windows: Chrome buffers the auth cookies in memory and writes them to the
/// store only when the browser closes, so wait for the cookie DB to go from
/// browser-held to released (the user closing the window), then read.
#[cfg(target_os = "windows")]
async fn wait_for_signin<F, Fut>(
    w: &SigninWait<'_>,
    _child: &mut Child,
    extract: &F,
) -> Result<String, String>
where
    F: Fn(Browser, PathBuf) -> Fut,
    Fut: Future<Output = Result<Option<String>, String>>,
{
    let started = Instant::now();
    let deadline = started + w.timeout;
    // Latches once the browser has opened the store, so a fresh empty profile
    // isn't read as "browser closed".
    let mut saw_browser = false;
    let mut last_extract_err: Option<String> = None;
    loop {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if Instant::now() > deadline {
            let detail = last_extract_err
                .as_deref()
                .map(|e| format!("; last extract error: {e}"))
                .unwrap_or_default();
            tracing::warn!(
                bin = w.bin,
                timeout_s = w.timeout.as_secs(),
                saw_browser,
                "sign-in timed out"
            );
            return Err(format!(
                "Sign-in not detected within {}s — finish signing in, then close the browser window{detail}",
                w.timeout.as_secs()
            ));
        }
        if super::windows_native::cookie_db_locked(w.profile) {
            saw_browser = true;
            continue;
        }
        if !saw_browser {
            continue;
        }
        match extract(w.browser, w.profile.to_path_buf()).await {
            Ok(Some(value)) => {
                tracing::info!(
                    bin = w.bin,
                    elapsed_ms = started.elapsed().as_millis(),
                    "sign-in detected after browser close"
                );
                return Ok(value);
            }
            Ok(None) => {}
            Err(e) => {
                if last_extract_err.as_deref() != Some(e.as_str()) {
                    tracing::trace!(error = %e, "cookie extract after close not ready");
                    last_extract_err = Some(e);
                }
            }
        }
    }
}
