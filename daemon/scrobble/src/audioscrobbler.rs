//! Shared Last.fm-compatible request signing and payloads.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize)]
pub struct TrackMetadata<'a> {
    pub(crate) artist_name: &'a str,
    pub(crate) track_name: &'a str,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) release_name: Option<&'a str>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) additional_info: Option<HashMap<&'a str, &'a str>>,
}

#[derive(Serialize)]
pub struct Scrobble<'a> {
    pub(crate) timestamp: i64,
    pub(crate) track_metadata: TrackMetadata<'a>,
}

#[derive(Serialize)]
pub struct NowPlaying<'a> {
    pub(crate) track_metadata: TrackMetadata<'a>,
}

#[derive(Deserialize)]
pub(crate) struct AuthTokenResponse {
    pub(crate) token: String,
}

#[derive(Deserialize)]
pub(crate) struct SessionResponse {
    pub(crate) session: Session,
}

#[derive(Deserialize)]
pub(crate) struct Session {
    pub(crate) key: String,
}

pub(crate) fn make_api_sig(params: &[(&str, &str)], api_secret: &str) -> String {
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
