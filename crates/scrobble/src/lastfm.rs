use crate::audioscrobbler;

pub use audioscrobbler::{
    NowPlaying, Scrobble, TrackMetadata, make_playing_now, make_scrobble, make_scrobble_at,
};

const API_URL: &str = "https://ws.audioscrobbler.com/2.0/";

pub fn auth_url(api_key: &str, token: &str) -> String {
    format!("https://www.last.fm/api/auth/?api_key={api_key}&token={token}")
}

pub async fn get_auth_token(api_key: &str) -> Result<String, reqwest::Error> {
    audioscrobbler::get_auth_token(API_URL, api_key).await
}

pub async fn get_session_key(
    api_key: &str,
    api_secret: &str,
    token: &str,
) -> Result<String, reqwest::Error> {
    audioscrobbler::get_session_key(API_URL, api_key, api_secret, token).await
}

pub async fn submit_scrobble(
    api_key: &str,
    api_secret: &str,
    session_key: &str,
    scrobble: &Scrobble<'_>,
) -> Result<String, reqwest::Error> {
    audioscrobbler::submit_scrobble(API_URL, api_key, api_secret, session_key, scrobble).await
}

pub async fn submit_now_playing(
    api_key: &str,
    api_secret: &str,
    session_key: &str,
    now_playing: &NowPlaying<'_>,
) -> Result<String, reqwest::Error> {
    audioscrobbler::submit_now_playing(API_URL, api_key, api_secret, session_key, now_playing).await
}
