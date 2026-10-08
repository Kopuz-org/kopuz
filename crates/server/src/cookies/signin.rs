use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use config::Browser;
use tokio::process::Child;

use super::browser::{
    BrowserBin, browser_candidates, find_browser_bin, in_flatpak, prepare_profile, signin_command,
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

    let bin = if let Ok(v) = std::env::var("KOPUZ_BROWSER_COMMAND")
        && !v.trim().is_empty()
    {
        tracing::info!("$KOPUZ_BROWSER_COMMAND used to override browser command");
        BrowserBin::CommandLine(v)
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
        find_browser_bin(browser, Some(profile.as_path()))
            .await
            .ok_or(error)?
    };
    tracing::info!(%bin, profile = %profile.display(), "launching sign-in browser");
    let mut child = signin_command(browser, &bin, &profile, signin_url)
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
    bin: &'a BrowserBin,
    timeout: Duration,
}

/// The store only goes on disk when the browser closes on Windows + Chromium:
/// Chrome buffers the auth cookies in memory until then. Everywhere else the
/// store is readable while the sign-in window is still open.
#[cfg(target_os = "windows")]
fn waits_for_close(browser: Browser) -> bool {
    browser.engine() == config::BrowserEngine::Chromium
}

#[cfg(not(target_os = "windows"))]
fn waits_for_close(_browser: Browser) -> bool {
    false
}

#[cfg(target_os = "windows")]
fn store_held_open(profile: &Path) -> bool {
    super::windows_native::cookie_db_locked(profile)
}

#[cfg(not(target_os = "windows"))]
fn store_held_open(_profile: &Path) -> bool {
    false
}

/// Poll `extract` every 500ms until it yields the signed-in value or the
/// deadline passes. Where the store is only written on close, the poll is
/// gated until the browser has both opened and released it — a fresh empty
/// profile must not read as "already closed".
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
    let gated = waits_for_close(w.browser);
    let mut saw_browser = false;
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
            tracing::warn!(
                bin = %w.bin,
                timeout_s = w.timeout.as_secs(),
                exited_early = child_exited_at.is_some(),
                saw_browser,
                "sign-in timed out"
            );
            if gated {
                return Err(format!(
                    "Sign-in not detected within {}s — finish signing in, then close the browser window{detail}",
                    w.timeout.as_secs()
                ));
            }
            let exited_note = child_exited_at
                .map(|_| " — note: the browser exited early (likely detached UI); close all browser windows and try again")
                .unwrap_or_default();
            return Err(format!(
                "Sign-in not detected within {}s{exited_note}{detail}",
                w.timeout.as_secs()
            ));
        }
        if gated {
            if store_held_open(w.profile) {
                saw_browser = true;
                continue;
            }
            if !saw_browser {
                continue;
            }
        } else if child_exited_at.is_none()
            && let Ok(Some(status)) = child.try_wait()
        {
            tracing::debug!(bin = %w.bin, %status, "browser exited — still polling cookies");
            child_exited_at = Some(Instant::now());
        }
        match extract(w.browser, w.profile.to_path_buf()).await {
            Ok(Some(value)) => {
                tracing::info!(
                    bin = %w.bin,
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
