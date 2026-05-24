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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn make_listen_sets_listened_at_and_filters_empty_release() {
        let listen = make_listen("Artist", "Track", Some(""));
        let serialized = serde_json::to_value(&listen).unwrap();

        assert!(listen.listened_at.is_some());
        assert!(serialized.get("listened_at").is_some());
        assert!(serialized["track_metadata"].get("release_name").is_none());
    }

    #[test]
    fn make_playing_now_omits_listened_at_and_keeps_release() {
        let now_playing = make_playing_now("Artist", "Track", Some("Album"));
        let serialized = serde_json::to_value(&now_playing).unwrap();

        assert!(now_playing.listened_at.is_none());
        assert_eq!(
            serialized["track_metadata"]["release_name"],
            Value::String("Album".to_string())
        );
        assert!(serialized.get("listened_at").is_none());
    }

    #[test]
    fn submit_listens_body_serializes_expected_shape() {
        let body = SubmitListens {
            listen_type: "single",
            payload: vec![make_listen("Artist", "Track", Some("Album"))],
        };

        let serialized = serde_json::to_value(&body).unwrap();

        assert_eq!(
            serialized["listen_type"],
            Value::String("single".to_string())
        );
        assert_eq!(serialized["payload"].as_array().map(|a| a.len()), Some(1));
        assert_eq!(
            serialized["payload"][0]["track_metadata"]["track_name"],
            Value::String("Track".to_string())
        );
    }
}
