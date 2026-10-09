//! Signing in with the YouTube session a browser on this machine already
//! holds, instead of signing in again in a window kopuz opens.

use std::path::{Path, PathBuf};
use std::time::Duration;

use config::Browser;

pub use crate::cookies::profiles::BrowserProfile;
use crate::cookies::{has_cookie, profiles};

use super::cookies::DOMAIN;

/// Present once Google has redirected a sign-in back to YouTube.
const SESSION: [&str; 2] = ["SID", "SAPISID"];

/// Where copies of a profile's cookie store go while they are read. Each copy
/// is deleted as soon as it has been.
pub fn scratch_dir() -> PathBuf {
    crate::cookies::profile_dir("browser-import", "")
}

/// How long one profile may take to be read and checked with YouTube before
/// it is left out of the list.
const CHECK_TIMEOUT: Duration = Duration::from_secs(15);

/// The browser profiles on this machine whose YouTube session YouTube Music
/// still accepts. A profile holding the session cookies can still hold a
/// session that was signed out elsewhere, so each one is read and checked,
/// all at once.
pub async fn signed_in(scratch: &Path) -> Vec<BrowserProfile> {
    let mut candidates = Vec::new();
    for profile in profiles::list() {
        match profiles::holds_cookies(&profile, scratch, DOMAIN, &SESSION).await {
            Ok(true) => candidates.push(profile),
            Ok(false) => {}
            Err(error) => tracing::debug!(
                browser = profile.browser.id(),
                %error,
                "could not check a browser profile for a YouTube session"
            ),
        }
    }
    let checks = candidates.into_iter().map(|profile| async move {
        let accepted = tokio::time::timeout(CHECK_TIMEOUT, async {
            match session_of(&profile, scratch).await {
                Ok(header) => crate::provider::validate_ytmusic_cookies(&header).await,
                Err(error) => {
                    tracing::debug!(browser = profile.browser.id(), %error, "could not read a browser profile's YouTube session");
                    false
                }
            }
        })
        .await
        .unwrap_or(false);
        if !accepted {
            tracing::debug!(browser = profile.browser.id(), "leaving out a browser profile YouTube Music does not accept");
        }
        accepted.then_some(profile)
    });
    let found: Vec<BrowserProfile> = futures_util::future::join_all(checks)
        .await
        .into_iter()
        .flatten()
        .collect();
    tracing::debug!(
        count = found.len(),
        "browser profiles signed in to YouTube Music"
    );
    found
}

/// The browser behind the profile [`BrowserProfile::id`] names, and the
/// `Cookie:` header of the YouTube Music session it holds.
pub async fn import(id: &str, scratch: &Path) -> Result<(Browser, String), String> {
    let profile = profiles::list()
        .into_iter()
        .find(|profile| profile.id() == id)
        .ok_or_else(|| "That browser profile is no longer there.".to_string())?;
    let header = session_of(&profile, scratch).await?;
    Ok((profile.browser, header))
}

async fn session_of(profile: &BrowserProfile, scratch: &Path) -> Result<String, String> {
    let cookies = profiles::read_cookies(profile, scratch, DOMAIN).await?;
    let header = super::cookies::header(&cookies);
    if !SESSION.iter().all(|name| has_cookie(&header, name)) {
        return Err(format!(
            "No YouTube Music session in {} ({}). Sign in to music.youtube.com there first.",
            profile.browser.label(),
            profile.name
        ));
    }
    Ok(header)
}

/// Reads the real browser profiles on this machine, copying their stores
/// under `target/` and deleting the copies. Lists the ones signed in to
/// YouTube by name; `KOPUZ_LIVE_IMPORT=<profile id>` also imports one and
/// checks the session with YouTube Music. No cookie value is printed.
#[cfg(test)]
mod live {
    use super::*;

    #[tokio::test]
    #[ignore = "reads the real browser profiles"]
    #[allow(clippy::print_stderr)]
    async fn lists_and_imports_real_profiles() {
        let scratch = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/live-import");
        for profile in profiles::list() {
            eprintln!(
                "live: profile {} / {} (account: {})",
                profile.browser.label(),
                profile.name,
                profile.email.is_some()
            );
        }
        let started = std::time::Instant::now();
        let found = signed_in(&scratch).await;
        eprintln!("live: checked in {} ms", started.elapsed().as_millis());
        for profile in &found {
            eprintln!("live: signed in: {}", profile.id());
        }
        if let Ok(id) = std::env::var("KOPUZ_LIVE_IMPORT") {
            let (browser, header) = import(&id, &scratch).await.expect("import");
            eprintln!(
                "live: imported from {}: {} cookies, {} bytes",
                browser.label(),
                header.split("; ").count(),
                header.len()
            );
            assert!(crate::provider::validate_ytmusic_cookies(&header).await);
            eprintln!("live: YouTube Music accepted the session");
        }
        let left = std::fs::read_dir(&scratch)
            .map(|dir| dir.count())
            .unwrap_or(0);
        assert_eq!(left, 0, "a copy was left behind");
    }
}
