use dioxus::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AvailableUpdate {
    pub version: String,
    pub release_url: String,
}

#[derive(serde::Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
}

fn parse_version_parts(version: &str) -> Option<Vec<u64>> {
    let core = version
        .trim()
        .trim_start_matches(['v', 'V'])
        .split(['-', '+'])
        .next()
        .unwrap_or_default();
    let parts: Option<Vec<u64>> = core
        .split('.')
        .map(|part| part.parse::<u64>().ok())
        .collect();
    parts.filter(|parts| !parts.is_empty())
}

fn is_newer_version(current: &str, candidate: &str) -> bool {
    let Some(current_parts) = parse_version_parts(current) else {
        return false;
    };
    let Some(candidate_parts) = parse_version_parts(candidate) else {
        return false;
    };

    let max_len = current_parts.len().max(candidate_parts.len());
    for idx in 0..max_len {
        let current_part = *current_parts.get(idx).unwrap_or(&0);
        let candidate_part = *candidate_parts.get(idx).unwrap_or(&0);
        match candidate_part.cmp(&current_part) {
            std::cmp::Ordering::Greater => return true,
            std::cmp::Ordering::Less => return false,
            std::cmp::Ordering::Equal => {}
        }
    }

    false
}

pub async fn fetch_available() -> Option<AvailableUpdate> {
    let client = reqwest::Client::builder()
        .user_agent(format!("kopuz/{}", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .ok()?;
    let release = client
        .get("https://api.github.com/repos/Kopuz-org/kopuz/releases/latest")
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json::<GithubRelease>()
        .await
        .ok()?;

    if is_newer_version(env!("CARGO_PKG_VERSION"), &release.tag_name) {
        Some(AvailableUpdate {
            version: release.tag_name.trim_start_matches(['v', 'V']).to_string(),
            release_url: release.html_url,
        })
    } else {
        None
    }
}

pub async fn run_rotation(mut config: Signal<config::AppConfig>) {
    let cookies = match config.peek().server.as_ref() {
        Some(s) if s.service == config::MusicService::YtMusic => s.access_token.clone(),
        _ => return,
    };
    let Some(cookies) = cookies else { return };
    if cookies.is_empty() {
        return;
    }
    let started = std::time::Instant::now();
    let from_len = cookies.len();
    let outcome =
        utils::offload(
            async move { server::ytmusic::verify_session_keepalive::tick(&cookies).await },
        )
        .await;
    match outcome {
        Ok(Some(updated)) => {
            tracing::debug!(
                secs = started.elapsed().as_secs_f32(),
                from = from_len,
                to = updated.len(),
                "verify_session OK - jar rotated",
            );
            if let Some(srv) = config.write().server.as_mut() {
                srv.access_token = Some(updated);
            }
        }
        Ok(None) => {}
        Err(e) => tracing::warn!(error = %e, "verify_session failed"),
    }
}

/// Refresh the active Spotify server's OAuth access token from its refresh token
/// (access tokens expire ~hourly) and write the re-packed pair back to config.
pub async fn run_spotify_refresh(mut config: Signal<config::AppConfig>) {
    let (packed, client_id) = match config.peek().server.as_ref() {
        Some(s) if s.service == config::MusicService::Spotify => {
            (s.access_token.clone(), s.url.clone())
        }
        _ => return,
    };
    let Some(packed) = packed else { return };
    if client_id.trim().is_empty() {
        return;
    }
    match server::spotify::auth::refresh_packed(&packed, client_id.clone()).await {
        Ok(new_packed) => {
            let mut cfg = config.write();
            if let Some(srv) = cfg.server.as_mut()
                && srv.service == config::MusicService::Spotify
                && srv.url == client_id
                && srv.access_token.as_deref() == Some(packed.as_str())
            {
                srv.access_token = Some(new_packed);
                tracing::debug!("spotify token refreshed");
            }
        }
        Err(e) => tracing::warn!(error = %e, "spotify token refresh failed"),
    }
}
