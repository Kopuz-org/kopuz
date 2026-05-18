use md5;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

const API_URL: &str = "https://ws.audioscrobbler.com/2.0/";

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
pub struct Scrobble<'a> {
    timestamp: i64,
    track_metadata: TrackMetadata<'a>,
}

#[derive(Serialize)]
pub struct NowPlaying<'a> {
    track_metadata: TrackMetadata<'a>,
}

#[derive(Deserialize)]
struct AuthTokenResponse {
    token: String,
}

#[derive(Deserialize)]
struct SessionResponse {
    session: Session,
}

#[derive(Deserialize)]
struct Session {
    key: String,
}

pub fn auth_url(api_key: &str, token: &str) -> String {
    format!(
        "https://www.last.fm/api/auth/?api_key={}&token={}",
        api_key, token
    )
}

fn make_api_sig(params: &[(&str, &str)], api_secret: &str) -> String {
    let mut sorted = params.to_vec();
    sorted.sort_by(|a, b| a.0.cmp(b.0));

    let mut sig = String::new();

    for (key, value) in sorted {
        sig.push_str(key);
        sig.push_str(value);
    }

    sig.push_str(api_secret);

    format!("{:x}", md5::compute(sig))
}

pub async fn get_auth_token(api_key: &str) -> Result<String, reqwest::Error> {
    let client = Client::new();

    let resp = client
        .get(API_URL)
        .query(&[
            ("method", "auth.getToken"),
            ("api_key", api_key),
            ("format", "json"),
        ])
        .send()
        .await?;

    resp.error_for_status_ref()?;

    let body: AuthTokenResponse = resp.json().await?;

    Ok(body.token)
}

pub async fn get_session_key(
    api_key: &str,
    api_secret: &str,
    token: &str,
) -> Result<String, reqwest::Error> {
    let client = Client::new();

    let params = [
        ("api_key", api_key),
        ("method", "auth.getSession"),
        ("token", token),
    ];

    let api_sig = make_api_sig(&params, api_secret);

    let resp = client
        .post(API_URL)
        .form(&[
            ("method", "auth.getSession"),
            ("api_key", api_key),
            ("token", token),
            ("api_sig", &api_sig),
            ("format", "json"),
        ])
        .send()
        .await?;

    resp.error_for_status_ref()?;

    let body: SessionResponse = resp.json().await?;

    Ok(body.session.key)
}

pub async fn submit_scrobble(
    api_key: &str,
    api_secret: &str,
    session_key: &str,
    scrobble: &Scrobble<'_>,
) -> Result<String, reqwest::Error> {
    let client = Client::new();

    let artist = scrobble.track_metadata.artist_name.trim();
    let track = scrobble.track_metadata.track_name.trim();
    let timestamp = scrobble.timestamp.to_string();
    let api_key = api_key.trim();
    let session_key = session_key.trim();

    let mut params: Vec<(&str, &str)> = vec![
        ("api_key", api_key),
        ("artist", artist),
        ("method", "track.scrobble"),
        ("sk", session_key),
        ("timestamp", &timestamp),
        ("track", track),
    ];

    if let Some(album) = scrobble.track_metadata.release_name {
        let album_trimmed = album.trim();
        if !album_trimmed.is_empty() {
            params.push(("album", album_trimmed));
        }
    }

    let api_sig = make_api_sig(&params, api_secret.trim());

    let mut form = params.clone();
    form.push(("api_sig", &api_sig));

    let url = format!("{}?format=json", API_URL);

    let resp = client
        .post(url)
        .form(&form)
        .send()
        .await?
        .error_for_status()?;
    Ok(resp.text().await?)
}

pub async fn submit_now_playing(
    api_key: &str,
    api_secret: &str,
    session_key: &str,
    now_playing: &NowPlaying<'_>,
) -> Result<String, reqwest::Error> {
    let client = Client::new();

    let artist = now_playing.track_metadata.artist_name.trim();
    let track = now_playing.track_metadata.track_name.trim();
    let api_key = api_key.trim();
    let session_key = session_key.trim();

    let mut params: Vec<(&str, &str)> = vec![
        ("api_key", api_key),
        ("artist", artist),
        ("method", "track.updateNowPlaying"),
        ("sk", session_key),
        ("track", track),
    ];

    if let Some(album) = now_playing.track_metadata.release_name {
        let album_trimmed = album.trim();
        if !album_trimmed.is_empty() {
            params.push(("album", album_trimmed));
        }
    }

    let api_sig = make_api_sig(&params, api_secret.trim());

    let mut form = params.clone();
    form.push(("api_sig", &api_sig));

    let url = format!("{}?format=json", API_URL);

    let resp = client
        .post(url)
        .form(&form)
        .send()
        .await?
        .error_for_status()?;
    Ok(resp.text().await?)
}

pub fn make_scrobble<'a>(
    artist: &'a str,
    track: &'a str,
    release: Option<&'a str>,
) -> Scrobble<'a> {
    let now_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    Scrobble {
        timestamp: now_unix,
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
) -> NowPlaying<'a> {
    NowPlaying {
        track_metadata: TrackMetadata {
            artist_name: artist,
            track_name: track,
            release_name: release.filter(|s| !s.is_empty()),
            additional_info: None,
        },
    }
}
