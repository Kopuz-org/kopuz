use std::collections::HashSet;
use std::time::Duration;

use reqwest::{Client, Method, Url};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::source::SourceError;

pub const BASE_URL: &str = "https://listener.clippsly.app";
pub const MAX_UPLOAD_BYTES: usize = 500 * 1024 * 1024;

#[derive(Clone)]
pub struct ClippslyClient {
    http: Client,
    base: Url,
    password: String,
}

#[derive(Deserialize)]
pub struct Account {
    pub id: u64,
}

#[derive(Debug, Deserialize)]
pub struct Track {
    pub id: u64,
    pub title: String,
    pub artist: String,
    pub genre: Option<String>,
    pub cover_art: Option<String>,
    pub duration_seconds: Option<f64>,
    pub album_id: Option<u64>,
    pub album_title: Option<String>,
    pub track_position: Option<u32>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct StorageQuota {
    pub used_bytes: u64,
    pub quota_bytes: u64,
    pub quota_remaining_bytes: u64,
}

#[derive(Deserialize)]
pub struct Stream {
    pub url: String,
    pub direct_url: Option<String>,
    pub token: Option<String>,
    pub bitrate: Option<u32>,
    pub duration_seconds: Option<f64>,
}

fn malformed() -> SourceError {
    SourceError::Backend("Clippsly returned an invalid response".into())
}

fn decode<T: DeserializeOwned>(value: Value) -> Result<T, SourceError> {
    serde_json::from_value(value).map_err(|_| malformed())
}

fn field<T: DeserializeOwned>(mut value: Value, key: &str) -> Result<T, SourceError> {
    decode(value.get_mut(key).ok_or_else(malformed)?.take())
}

fn item_id(value: &str) -> Result<u64, SourceError> {
    value
        .parse()
        .map_err(|_| SourceError::InvalidInput("invalid Clippsly item id".into()))
}

pub fn validate_upload(filename: &str, length: usize) -> Result<(), SourceError> {
    let extension = filename
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if filename.is_empty() || filename.contains(['/', '\\', '\r', '\n', '\0']) {
        return Err(SourceError::InvalidInput("invalid upload filename".into()));
    }
    if !matches!(
        extension.as_str(),
        "mp3" | "m4a" | "flac" | "wav" | "ogg" | "opus" | "wma" | "aiff"
    ) {
        return Err(SourceError::InvalidInput("unsupported audio format".into()));
    }
    if length == 0 || length > MAX_UPLOAD_BYTES {
        return Err(SourceError::InvalidInput(
            "audio files must be between 1 byte and 500 MiB".into(),
        ));
    }
    Ok(())
}

impl ClippslyClient {
    pub fn new(base: &str, password: &str) -> Result<Self, SourceError> {
        let base =
            Url::parse(&format!("{}/", base.trim_end_matches('/'))).map_err(|_| malformed())?;
        if !matches!(base.scheme(), "http" | "https")
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err(SourceError::InvalidInput("invalid Clippsly URL".into()));
        }
        let http = Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| SourceError::Connectivity)?;
        Ok(Self {
            http,
            base,
            password: password.to_string(),
        })
    }

    fn request(&self, method: Method, path: &str) -> Result<reqwest::RequestBuilder, SourceError> {
        let url = self.base.join(path).map_err(|_| malformed())?;
        Ok(self.http.request(method, url).bearer_auth(&self.password))
    }

    async fn response(request: reqwest::RequestBuilder) -> Result<Value, SourceError> {
        let response = request
            .send()
            .await
            .map_err(|_| SourceError::Connectivity)?;
        match response.status().as_u16() {
            200..=299 => {}
            401 | 403 => return Err(SourceError::Auth),
            413 => {
                return Err(SourceError::InvalidInput(
                    "Clippsly refused the upload size".into(),
                ));
            }
            415 => {
                return Err(SourceError::InvalidInput(
                    "Clippsly refused the audio format".into(),
                ));
            }
            429 => {
                return Err(SourceError::Backend(
                    "Clippsly rate limit reached; try again later".into(),
                ));
            }
            code => {
                return Err(SourceError::Backend(format!(
                    "Clippsly request failed (HTTP {code})"
                )));
            }
        }
        if response.status() == reqwest::StatusCode::NO_CONTENT {
            return Ok(Value::Null);
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|_| SourceError::Connectivity)?;
        if bytes.is_empty() {
            return Ok(Value::Null);
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| malformed())?;
        if value.get("success") == Some(&Value::Bool(false)) || value.get("error").is_some() {
            return Err(SourceError::Backend("Clippsly rejected the request".into()));
        }
        Ok(value)
    }

    async fn get(&self, path: &str) -> Result<Value, SourceError> {
        Self::response(self.request(Method::GET, path)?).await
    }

    pub async fn account(&self) -> Result<Account, SourceError> {
        field(self.get("v1/account").await?, "account")
    }

    pub async fn storage(&self) -> Result<StorageQuota, SourceError> {
        decode(self.get("v1/account/storage").await?)
    }

    pub async fn tracks(&self) -> Result<Vec<Track>, SourceError> {
        let mut tracks = Vec::new();
        let mut seen = HashSet::new();
        loop {
            let response = self
                .get(&format!(
                    "v1/library/tracks?limit=200&offset={}",
                    tracks.len()
                ))
                .await?;
            let page: Vec<Track> = field(response, "tracks")?;
            let count = page.len();
            for track in page {
                if !seen.insert(track.id) {
                    return Err(SourceError::Backend(
                        "Clippsly repeated a track while paging the library".into(),
                    ));
                }
                tracks.push(track);
            }
            if count < 200 {
                return Ok(tracks);
            }
        }
    }

    pub async fn stream(&self, id: &str) -> Result<Stream, SourceError> {
        let id = item_id(id)?;
        field(
            self.get(&format!("v1/tracks/{id}/stream?variant=best"))
                .await?,
            "stream",
        )
    }

    pub fn media_url(&self, url: &str) -> Option<String> {
        if url.trim().is_empty() {
            return None;
        }
        let url = self.base.join(url).ok()?;
        (matches!(url.scheme(), "http" | "https")
            && url.username().is_empty()
            && url.password().is_none())
        .then(|| url.to_string())
    }

    pub fn stream_url(&self, stream: &Stream) -> Result<String, SourceError> {
        if let Some(direct) = stream.direct_url.as_deref().filter(|url| !url.is_empty()) {
            return self.media_url(direct).ok_or_else(malformed);
        }
        let mut url = Url::parse(&self.media_url(&stream.url).ok_or_else(malformed)?)
            .map_err(|_| malformed())?;
        if let Some(token) = &stream.token
            && !url.query_pairs().any(|(key, _)| key == "token")
        {
            url.query_pairs_mut().append_pair("token", token);
        }
        Ok(url.to_string())
    }

    pub async fn upload(&self, filename: String, content: Vec<u8>) -> Result<(), SourceError> {
        validate_upload(&filename, content.len())?;
        let quota = self.storage().await?;
        if content.len() as u64 > quota.quota_remaining_bytes {
            return Err(SourceError::InvalidInput(
                "the file exceeds the remaining Clippsly storage quota".into(),
            ));
        }
        let audio = reqwest::multipart::Part::bytes(content).file_name(filename);
        Self::response(
            self.request(Method::POST, "v1/upload")?
                .timeout(Duration::from_secs(600))
                .multipart(reqwest::multipart::Form::new().part("audio", audio)),
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
