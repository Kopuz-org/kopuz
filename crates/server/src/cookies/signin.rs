use std::future::Future;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use config::Browser;

use super::browser::{
    browser_candidates, browser_command, find_browser_bin, find_host_browser_bin, in_flatpak,
};
use super::profile::profile_dir;

/// Wipe the `<prefix>-<server_id>` profile, launch `browser` at `signin_url`,
/// then poll via `extract` until it yields the signed-in result. `extract`
/// returns `Ok(Some(value))` when done, `Ok(None)` while pending, `Err` for a
/// transient read error (logged, retried). The browser is always killed before
/// returning.
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

    let bin = if in_flatpak() {
        find_host_browser_bin(browser).await.ok_or_else(|| {
            format!(
                "{browser} not found on the host (looked for: {}). Install it on the host system.",
                browser_candidates(browser).join(", ")
            )
        })?
    } else {
        find_browser_bin(browser).ok_or_else(|| {
            format!(
                "{browser} not found in PATH (looked for: {}). Install it, or set $KOPUZ_{}_BIN.",
                browser_candidates(browser).join(", "),
                browser.id().to_uppercase().replace('-', "_")
            )
        })?
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
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0100_0000);
    }
    let mut child = cmd
        .arg(signin_url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("spawn {bin}: {e}"))?;
    tracing::debug!(%bin, pid = ?child.id(), "browser spawned — polling for sign-in cookies");

    let started = Instant::now();
    let deadline = started + signin_timeout;
    let mut last_extract_err: Option<String> = None;
    // Edge/Chrome on Windows often spawn the UI detached and the launched parent
    // exits ~0 in <1s. The cookie store is still on disk in our profile, so keep
    // polling regardless of child exit; just note it for the timeout message.
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
            tracing::warn!(%bin, timeout_s = signin_timeout.as_secs(), exited_early = child_exited_at.is_some(), "sign-in timed out");
            break Err(format!(
                "Sign-in not detected within {}s{exited_note}{detail}",
                signin_timeout.as_secs()
            ));
        }
        if child_exited_at.is_none()
            && let Ok(Some(status)) = child.try_wait()
        {
            tracing::debug!(%bin, %status, "browser exited — still polling cookies (may be a detached UI)");
            child_exited_at = Some(Instant::now());
        }
        match extract(browser, profile.clone()).await {
            Ok(Some(value)) => {
                tracing::info!(%bin, elapsed_ms = started.elapsed().as_millis(), "sign-in detected — closing browser");
                break Ok(value);
            }
            Ok(None) => {}
            Err(e) => {
                // Usually just "cookie store not ready yet" while the user is
                // still signing in; log once per distinct message to avoid spam.
                if last_extract_err.as_deref() != Some(e.as_str()) {
                    tracing::trace!(error = %e, "cookie extract not ready yet");
                    last_extract_err = Some(e);
                }
            }
        }
    };

    let _ = child.kill().await;
    outcome
}
