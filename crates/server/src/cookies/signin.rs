use std::path::Path;
use std::time::{Duration, Instant};

use config::{Browser, BrowserEngine};
use tokio::process::Child;

use super::browser::{
    BrowserBin, browser_candidates, find_browser_bin, in_flatpak, prepare_profile, signin_command,
    spawn_browser,
};
use super::cdp::{self, Cdp};
use super::profile::profile_dir;
use super::store::{Cookie, read_cookies};

/// Wipe the `<prefix>-<server_id>` profile, launch `browser` at `signin_url`,
/// then hand the profile's `domain` cookies to `extract` until it yields the
/// signed-in result. A Chromium browser is read over the DevTools pipe while
/// that holds; otherwise, and once the pipe is gone, the store is read from
/// disk (transient read errors are logged and retried). The browser is always
/// stopped before returning.
pub(crate) async fn launch_signin_and_extract<F>(
    browser: Browser,
    server_id: &str,
    prefix: &str,
    signin_url: &str,
    domain: &str,
    signin_timeout: Duration,
    extract: F,
) -> Result<String, String>
where
    F: Fn(&[Cookie]) -> Option<String>,
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
    let mut command = signin_command(browser, &bin, &profile, signin_url);
    let pipe = if browser.engine() == BrowserEngine::Chromium && cdp::pipe_reaches(&bin) {
        match cdp::pipes() {
            Ok((devtools, ends)) => {
                ends.attach(&mut command);
                Some((devtools, ends))
            }
            Err(e) => {
                tracing::warn!(error = %e, "no DevTools pipe; reading the cookie store from disk");
                None
            }
        }
    } else {
        None
    };
    let mut child = spawn_browser(&mut command).map_err(|e| format!("spawn {bin}: {e}"))?;
    let mut devtools = pipe.map(|(devtools, ends)| {
        drop(ends);
        devtools
    });
    tracing::debug!(%bin, pid = ?child.id(), devtools = devtools.is_some(), "browser spawned, waiting for sign-in");

    let wait = SigninWait {
        browser,
        profile: &profile,
        bin: &bin,
        url: signin_url,
        domain,
        timeout: signin_timeout,
    };
    let outcome = wait_for_signin(&wait, &mut child, &mut devtools, &extract).await;

    match devtools.as_mut() {
        // A browser asked to quit writes its cookies out first, which is
        // what a later resume from this profile reads.
        Some(devtools) => {
            devtools.close().await;
            if tokio::time::timeout(Duration::from_secs(3), child.wait())
                .await
                .is_err()
            {
                let _ = child.kill().await;
            }
        }
        None => {
            let _ = child.kill().await;
        }
    }
    outcome
}

/// Fixed inputs for a sign-in wait, shared by both platform strategies.
struct SigninWait<'a> {
    browser: Browser,
    profile: &'a Path,
    bin: &'a BrowserBin,
    url: &'a str,
    domain: &'a str,
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

/// Poll the cookies every 500ms until `extract` yields the signed-in value or
/// the deadline passes. The DevTools pipe answers from the browser's memory,
/// so while it holds nothing waits on the disk. Without it, where the store is
/// only written on close, the poll is gated until the browser has both opened
/// and released it: a fresh empty profile must not read as "already closed".
async fn wait_for_signin<F>(
    w: &SigninWait<'_>,
    child: &mut Child,
    devtools: &mut Option<Cdp>,
    extract: &F,
) -> Result<String, String>
where
    F: Fn(&[Cookie]) -> Option<String>,
{
    let started = Instant::now();
    let deadline = started + w.timeout;
    let gated = waits_for_close(w.browser);
    let mut saw_browser = false;
    let mut answered = false;
    let mut nudges = [Duration::from_secs(5), Duration::from_secs(15)]
        .into_iter()
        .peekable();
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
            if gated && devtools.is_none() {
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
        if let Some(pipe) = devtools.as_mut() {
            if nudges
                .next_if(|after| started.elapsed() >= *after)
                .is_some()
            {
                pipe.renavigate_if_stalled(w.url).await;
            }
            match pipe.cookies(w.domain).await {
                Ok(cookies) => {
                    saw_browser = true;
                    answered = true;
                    if let Some(value) = extract(&cookies) {
                        tracing::info!(
                            bin = %w.bin,
                            elapsed_ms = started.elapsed().as_millis(),
                            "sign-in detected over DevTools"
                        );
                        return Ok(value);
                    }
                    continue;
                }
                // The browser that held the pipe is gone, and its cookies
                // were read a moment ago: the store on disk holds no more.
                Err(e)
                    if answered
                        && matches!(
                            e.kind(),
                            std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::BrokenPipe
                        ) =>
                {
                    tracing::info!(bin = %w.bin, "sign-in browser closed before signing in");
                    return Err(format!(
                        "{} closed before sign-in finished. Try again.",
                        w.browser.label()
                    ));
                }
                // Never answered, or stopped answering: the browser took no
                // pipe, or it is stuck. Whatever it wrote is on disk.
                Err(e) => {
                    tracing::debug!(error = %e, "DevTools pipe unusable; reading the cookie store from disk");
                    *devtools = None;
                }
            }
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
        match read_cookies(w.browser, w.profile, w.domain).await {
            Ok(cookies) => {
                if let Some(value) = extract(&cookies) {
                    tracing::info!(
                        bin = %w.bin,
                        elapsed_ms = started.elapsed().as_millis(),
                        "sign-in detected"
                    );
                    return Ok(value);
                }
            }
            Err(e) => {
                if last_extract_err.as_deref() != Some(e.as_str()) {
                    tracing::trace!(error = %e, "cookie extract not ready yet");
                    last_extract_err = Some(e);
                }
            }
        }
    }
}

/// Runs against a real browser: `KOPUZ_LIVE_BROWSER` picks it (Helium
/// otherwise). It opens YouTube Music signed out in a throwaway profile under
/// `target/` and waits for the visitor cookies the page sets.
#[cfg(test)]
mod live {
    use std::path::PathBuf;

    use super::*;

    #[tokio::test]
    #[ignore = "opens a real browser"]
    #[allow(clippy::print_stderr)]
    async fn visitor_cookies_come_back_over_the_pipe() {
        let id = std::env::var("KOPUZ_LIVE_BROWSER").unwrap_or_else(|_| "helium".to_string());
        let browser = Browser::from_id(&id).expect("a browser kopuz knows");
        let profile = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/live-signin")
            .join(browser.id());
        let _ = std::fs::remove_dir_all(&profile);
        std::fs::create_dir_all(&profile).expect("profile dir");
        let bin = find_browser_bin(browser, Some(&profile))
            .await
            .expect("browser installed");
        assert!(cdp::pipe_reaches(&bin));

        let mut command = signin_command(browser, &bin, &profile, "https://music.youtube.com/");
        let (devtools, ends) = cdp::pipes().expect("pipes");
        ends.attach(&mut command);
        let mut child = spawn_browser(&mut command).expect("spawn");
        drop(ends);
        let mut devtools = Some(devtools);
        let wait = SigninWait {
            browser,
            profile: &profile,
            bin: &bin,
            url: "https://music.youtube.com/",
            domain: "youtube.com",
            timeout: Duration::from_secs(60),
        };
        let found = wait_for_signin(&wait, &mut child, &mut devtools, &|cookies: &[Cookie]| {
            cookies
                .iter()
                .any(|c| c.name == "VISITOR_INFO1_LIVE" || c.name == "YSC")
                .then(|| {
                    cookies
                        .iter()
                        .map(|c| c.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
        })
        .await;
        let read_over_pipe = devtools.is_some();
        if let Some(devtools) = devtools.as_mut() {
            devtools.close().await;
        }
        let _ = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;
        let _ = child.kill().await;
        let names = found.expect("visitor cookies");
        eprintln!("live: {} cookies over the pipe: {names}", browser.label());
        assert!(read_over_pipe, "fell back to the disk store");

        // What a resume from this profile reads once the window is gone: the
        // persistent cookies, from a headless start on the same profile.
        let args = ["--password-store=basic".to_string()];
        let stored = cdp::read_headless(&bin, &profile, &args, "youtube.com")
            .await
            .expect("headless read");
        let names: Vec<_> = stored.iter().map(|c| c.name.as_str()).collect();
        eprintln!(
            "live: {} cookies read headless: {}",
            browser.label(),
            names.join(", ")
        );
        assert!(!stored.is_empty());
        let _ = std::fs::remove_dir_all(&profile);
    }
}
