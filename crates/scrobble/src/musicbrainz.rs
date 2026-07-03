use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_ATTEMPTS: u32 = 5;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_BACKOFF: Duration = Duration::from_secs(60);

#[derive(Serialize)]
pub struct TrackMetadata<'a> {
    artist_name: &'a str,
    track_name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    release_name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    additional_info: Option<HashMap<&'a str, &'a str>>,
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

pub async fn validate_token(token: &str) -> Result<Option<String>, reqwest::Error> {
    let client = Client::new();
    let url = "https://api.listenbrainz.org/1/validate-token";

    let resp = client
        .get(url)
        .header("Authorization", token)
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
) -> Result<reqwest::Response, reqwest::Error> {
    let client = Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .unwrap_or_else(|_| Client::new());
    let url = "https://api.listenbrainz.org/1/submit-listens";
    let body = SubmitListens {
        listen_type,
        payload: listens,
    };

    let mut attempt: u32 = 0;
    loop {
        attempt += 1;

        let result = client
            .post(url)
            .header("Authorization", token)
            .json(&body)
            .send()
            .await;

        let resp = match result {
            Ok(resp) => resp,
            Err(error) => {
                if attempt >= MAX_ATTEMPTS || !is_retryable_error(&error) {
                    return Err(error);
                }
                tracing::warn!(
                    "ListenBrainz {listen_type} transport error (attempt {attempt}/{MAX_ATTEMPTS}), retrying: {error}"
                );
                tokio::time::sleep(backoff_delay(attempt, None)).await;
                continue;
            }
        };

        let status = resp.status();

        if status.is_success() {
            return Ok(resp);
        }

        let retryable =
            status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error();

        if !retryable || attempt >= MAX_ATTEMPTS {
            resp.error_for_status_ref()?;
            return Ok(resp);
        }

        let retry_after = parse_retry_after(&resp);
        tracing::warn!(
            "ListenBrainz {listen_type} HTTP {status} (attempt {attempt}/{MAX_ATTEMPTS}), retrying"
        );
        tokio::time::sleep(backoff_delay(attempt, retry_after)).await;
    }
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

pub fn make_listen<'a>(
    artist: &'a str,
    track: &'a str,
    release: Option<&'a str>,
    additional_info: Option<HashMap<&'a str, &'a str>>,
) -> Listen<'a> {
    let now_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    Listen {
        listened_at: Some(now_unix),
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
    additional_info: Option<HashMap<&'a str, &'a str>>,
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
