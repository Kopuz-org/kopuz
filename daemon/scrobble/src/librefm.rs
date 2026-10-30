use crate::audioscrobbler::{
    AuthTokenResponse, NowPlaying, Scrobble, SessionResponse, TrackMetadata, make_api_sig,
};
use std::time::{SystemTime, UNIX_EPOCH};

const API_URL: &str = "https://libre.fm/2.0/";

/// Libre.fm accepts a fixed client identifier without per-app registration.
pub const API_KEY: &str = "kopuz";
pub const API_SECRET: &str = "kopuz";

pub fn auth_url(api_key: &str, token: &str) -> String {
    format!("https://libre.fm/api/auth/?api_key={api_key}&token={token}")
}

pub async fn get_auth_token(api_key: &str) -> Result<String, reqwest::Error> {
    let client = utils::http::client();

    let resp = client
        .get(API_URL)
        .query(&[
            ("method", "auth.getToken"),
            ("api_key", api_key),
            ("format", "json"),
        ])
        .send()
        .await
        .map_err(reqwest::Error::without_url)?;

    resp.error_for_status_ref()
        .map_err(reqwest::Error::without_url)?;

    let body: AuthTokenResponse = resp.json().await.map_err(reqwest::Error::without_url)?;

    Ok(body.token)
}

pub async fn get_session_key(
    api_key: &str,
    api_secret: &str,
    token: &str,
) -> Result<String, reqwest::Error> {
    let client = utils::http::client();

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
    let client = utils::http::client();

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
    resp.text().await
}

pub async fn submit_now_playing(
    api_key: &str,
    api_secret: &str,
    session_key: &str,
    now_playing: &NowPlaying<'_>,
) -> Result<String, reqwest::Error> {
    let client = utils::http::client();

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
    resp.text().await
}

pub fn make_scrobble<'a>(
    artist: &'a str,
    track: &'a str,
    release: Option<&'a str>,
) -> Scrobble<'a> {
    let now_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    make_scrobble_at(artist, track, release, now_unix)
}

/// Like [`make_scrobble`], but with an explicit listen timestamp. Used when
/// resubmitting queued offline scrobbles so they keep their original time.
pub fn make_scrobble_at<'a>(
    artist: &'a str,
    track: &'a str,
    release: Option<&'a str>,
    timestamp: i64,
) -> Scrobble<'a> {
    Scrobble {
        timestamp,
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
