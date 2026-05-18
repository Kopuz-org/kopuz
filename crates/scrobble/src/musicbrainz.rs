use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

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
    let client = Client::new();
    let url = "https://api.listenbrainz.org/1/submit-listens";
    let body = SubmitListens {
        listen_type,
        payload: listens,
    };

    let resp = client
        .post(url)
        .header("Authorization", token)
        .json(&body)
        .send()
        .await?;

    resp.error_for_status_ref()?;

    Ok(resp)
}

pub fn make_listen<'a>(artist: &'a str, track: &'a str, release: Option<&'a str>) -> Listen<'a> {
    let now_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    Listen {
        listened_at: Some(now_unix),
        track_metadata: TrackMetadata {
            artist_name: artist,
            track_name: track,
            release_name: release.filter(|s| !s.is_empty()),
            additional_info: None,
        },
    }
}

pub fn make_playing_now<'a>(
    artist: &'a str,
    track: &'a str,
    release: Option<&'a str>,
) -> Listen<'a> {
    Listen {
        listened_at: None,
        track_metadata: TrackMetadata {
            artist_name: artist,
            track_name: track,
            release_name: release.filter(|s| !s.is_empty()),
            additional_info: None,
        },
    }
}
