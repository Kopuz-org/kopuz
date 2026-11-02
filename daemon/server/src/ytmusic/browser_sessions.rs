//! Signing in with the YouTube session a browser on this machine already holds.

use std::path::{Path, PathBuf};
use std::time::Duration;

use config::Browser;

pub use crate::cookies::profiles::BrowserProfile;
use crate::cookies::{has_cookie, profiles};

use super::cookies::DOMAIN;

/// Present once Google has redirected a sign-in back to YouTube.
const SESSION: [&str; 2] = ["SID", "SAPISID"];

pub fn scratch_dir() -> PathBuf {
    crate::cookies::profile_dir("browser-import", "")
}

/// A profile slower than this is left out of the list.
const CHECK_TIMEOUT: Duration = Duration::from_secs(15);

/// Session cookies can outlive a sign-out elsewhere, so each profile is checked
/// with YouTube Music, all at once.
pub async fn signed_in(scratch: &Path) -> Vec<BrowserProfile> {
    let mut candidates = Vec::new();
    for profile in profiles::browser_profiles() {
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
                Ok(header) => crate::provider::check_ytmusic_session(&header)
                    .await
                    .is_usable(),
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

/// Why a profile's session could not be taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportError {
    ProfileGone,
    CookiesUnreadable(String),
    SignedOut { browser: String, profile: String },
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ProfileGone => f.write_str("That browser profile is no longer there."),
            Self::CookiesUnreadable(error) => f.write_str(error),
            Self::SignedOut { browser, profile } => write!(
                f,
                "No YouTube Music session in {browser} ({profile}). Sign in to music.youtube.com there first."
            ),
        }
    }
}

pub async fn import(id: &str, scratch: &Path) -> Result<(Browser, String), ImportError> {
    let profile = profiles::browser_profiles()
        .into_iter()
        .find(|profile| profile.id() == id)
        .ok_or(ImportError::ProfileGone)?;
    let header = session_of(&profile, scratch).await?;
    Ok((profile.browser, header))
}

async fn session_of(profile: &BrowserProfile, scratch: &Path) -> Result<String, ImportError> {
    let cookies = profiles::read_cookies(profile, scratch, DOMAIN)
        .await
        .map_err(ImportError::CookiesUnreadable)?;
    let header = super::cookies::header(&cookies);
    if !SESSION.iter().all(|name| has_cookie(&header, name)) {
        return Err(ImportError::SignedOut {
            browser: profile.browser.label().to_string(),
            profile: profile.name.clone(),
        });
    }
    Ok(header)
}

/// Reads the real profiles on this machine; `KOPUZ_LIVE_IMPORT=<profile id>` also
/// imports one. No cookie value is printed.
#[cfg(test)]
mod live {
    use super::*;

    #[tokio::test]
    #[ignore = "reads the real browser profiles"]
    #[allow(clippy::print_stderr)]
    async fn lists_and_imports_real_profiles() {
        let scratch = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/live-import");
        for profile in profiles::browser_profiles() {
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
            assert_eq!(
                crate::provider::check_ytmusic_session(&header).await,
                crate::ytmusic::SessionCheck::Valid
            );
            eprintln!("live: YouTube Music accepted the session");
        }
        let left = std::fs::read_dir(&scratch)
            .map(|dir| dir.count())
            .unwrap_or(0);
        assert_eq!(left, 0, "a copy was left behind");
    }
}
