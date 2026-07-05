use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_ATTEMPTS: u32 = 6;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_BACKOFF: Duration = Duration::from_secs(60);
// playing_now is ephemeral: the next heartbeat replaces it, so retrying a
// stale submission only delays everything queued behind it.
const PLAYING_NOW_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Serialize)]
pub struct TrackMetadata<'a> {
    artist_name: &'a str,
    track_name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    release_name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    additional_info: Option<HashMap<&'a str, serde_json::Value>>,
}

#[derive(Serialize)]
pub struct Listen<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    listened_at: Option<i64>,
    track_metadata: TrackMetadata<'a>,
}

#[derive(Serialize)]
pub struct SubmitListens<'a> {
    listen_type: &'a str,
    payload: Vec<Listen<'a>>,
}

#[derive(Deserialize)]
struct ValidateResponse {
    valid: bool,
    user_name: Option<String>,
}

pub fn auth_header(token: &str) -> String {
    let token = token.trim();
    if token.contains(' ') {
        token.to_string()
    } else {
        format!("Token {token}")
    }
}

pub async fn validate_token(token: &str) -> Result<Option<String>, reqwest::Error> {
    let client = Client::new();
    let url = "https://api.listenbrainz.org/1/validate-token";

    let resp = client
        .get(url)
        .header("Authorization", auth_header(token))
        .send()
        .await?;

    resp.error_for_status_ref()?;

    let body: ValidateResponse = resp.json().await?;

    if body.valid {
        Ok(body.user_name)
    } else {
        Ok(None)
    }
}

pub async fn submit_listens(
    token: &str,
    listens: Vec<Listen<'_>>,
    listen_type: &str,
) -> Result<(), reqwest::Error> {
    let is_playing_now = listen_type == "playing_now";
    let max_attempts = if is_playing_now { 1 } else { MAX_ATTEMPTS };
    let timeout = if is_playing_now {
        PLAYING_NOW_TIMEOUT
    } else {
        REQUEST_TIMEOUT
    };
    let client = Client::builder()
        .timeout(timeout)
        .build()
        .unwrap_or_else(|_| Client::new());
    let url = "https://api.listenbrainz.org/1/submit-listens";
    let auth = auth_header(token);
    let count = listens.len();
    let body = SubmitListens {
        listen_type,
        payload: listens,
    };

    let mut attempt: u32 = 0;
    loop {
        attempt += 1;

        let result = client
            .post(url)
            .header("Authorization", auth.as_str())
            .json(&body)
            .send()
            .await;

        let resp = match result {
            Ok(resp) => resp,
            Err(error) => {
                let kind = error_kind(&error);
                if attempt >= max_attempts || !is_retryable_error(&error) {
                    if is_playing_now {
                        tracing::debug!(
                            "ListenBrainz {listen_type} ({count}) skipped, next heartbeat will resend: {kind}: {error}"
                        );
                    } else {
                        tracing::warn!(
                            "ListenBrainz {listen_type} ({count}) failed after {attempt} attempt(s): {kind}: {error}"
                        );
                    }
                    return Err(error);
                }
                let delay = backoff_delay(attempt, None);
                tracing::warn!(
                    "ListenBrainz {listen_type} ({count}) {kind} on attempt {attempt}/{max_attempts}, retrying in {delay:?}: {error}"
                );
                tokio::time::sleep(delay).await;
                continue;
            }
        };

        let status = resp.status();

        if status.is_success() {
            let body = read_body(resp).await;
            if listen_type == "playing_now" {
                tracing::debug!(
                    "ListenBrainz {listen_type} ({count}) accepted: HTTP {} {body}",
                    status.as_u16()
                );
            } else {
                tracing::info!(
                    "ListenBrainz {listen_type} ({count}) accepted: HTTP {} {body}",
                    status.as_u16()
                );
            }
            return Ok(());
        }

        let retryable =
            status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
        let retry_after = parse_retry_after(&resp);

        if !retryable || attempt >= max_attempts {
            let error = resp.error_for_status_ref().unwrap_err();
            let body = read_body(resp).await;
            tracing::warn!(
                "ListenBrainz {listen_type} ({count}) rejected after {attempt} attempt(s): HTTP {} {body}",
                status.as_u16()
            );
            return Err(error);
        }

        let delay = backoff_delay(attempt, retry_after);
        let body = read_body(resp).await;
        tracing::warn!(
            "ListenBrainz {listen_type} ({count}) HTTP {} on attempt {attempt}/{max_attempts}, retrying in {delay:?} {body}",
            status.as_u16()
        );
        tokio::time::sleep(delay).await;
    }
}

fn error_kind(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "timeout"
    } else if error.is_connect() {
        "connection error"
    } else if error.is_request() {
        "request error"
    } else {
        "transport error"
    }
}

async fn read_body(resp: reqwest::Response) -> String {
    let text = resp.text().await.unwrap_or_default();
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let snippet: String = trimmed.chars().take(300).collect();
    format!("body={snippet}")
}

fn is_retryable_error(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect() || error.is_request()
}

fn parse_retry_after(resp: &reqwest::Response) -> Option<Duration> {
    let secs = resp
        .headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?;
    Some(Duration::from_secs(secs))
}

fn backoff_delay(attempt: u32, retry_after: Option<Duration>) -> Duration {
    let base = Duration::from_secs(1u64 << (attempt - 1).min(6));
    retry_after.unwrap_or(base).min(MAX_BACKOFF)
}

/// Unix timestamp for "now"; capture when playback starts and pass to `make_listen`.
pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub fn make_listen<'a>(
    artist: &'a str,
    track: &'a str,
    release: Option<&'a str>,
    additional_info: Option<HashMap<&'a str, serde_json::Value>>,
    listened_at: i64,
) -> Listen<'a> {
    Listen {
        listened_at: Some(listened_at),
        track_metadata: TrackMetadata {
            artist_name: artist,
            track_name: track,
            release_name: release.filter(|s| !s.is_empty()),
            additional_info,
        },
    }
}

pub fn make_playing_now<'a>(
    artist: &'a str,
    track: &'a str,
    release: Option<&'a str>,
    additional_info: Option<HashMap<&'a str, serde_json::Value>>,
) -> Listen<'a> {
    Listen {
        listened_at: None,
        track_metadata: TrackMetadata {
            artist_name: artist,
            track_name: track,
            release_name: release.filter(|s| !s.is_empty()),
            additional_info,
        },
    }
}
