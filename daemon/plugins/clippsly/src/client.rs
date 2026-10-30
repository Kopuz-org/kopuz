use std::collections::HashSet;
use std::time::Duration;

use kopuz_plugin_sdk::SourceError;
use kopuz_plugin_sdk::http::{Client, RequestBuilder};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use url::Url;

pub const MAX_UPLOAD_BYTES: usize = 500 * 1024 * 1024;

/// Tracks per library page.
const PAGE: usize = 200;

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
    if filename.is_empty() || filename.contains(['/', '\\', '\r', '\n', '\0', '"']) {
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

/// A `multipart/form-data` body with one file part, and its content type.
fn multipart(field: &str, filename: &str, content: &[u8]) -> (Vec<u8>, String) {
    // The boundary only has to be absent from the payload; a validated name
    // never holds quotes or line breaks, and the length check is cheap.
    let mut boundary = String::from("kopuz-clippsly-upload");
    while content
        .windows(boundary.len())
        .any(|window| window == boundary.as_bytes())
    {
        boundary.push('x');
    }
    let mut body = Vec::with_capacity(content.len() + 256);
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{field}\"; filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(content);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (body, format!("multipart/form-data; boundary={boundary}"))
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
        Ok(Self {
            http: Client::new().with_timeout(Duration::from_secs(60)),
            base,
            password: password.to_string(),
        })
    }

    fn request(&self, method: &str, path: &str) -> Result<RequestBuilder, SourceError> {
        let url = self.base.join(path).map_err(|_| malformed())?;
        Ok(self.http.request(method, url).bearer_auth(&self.password))
    }

    fn response(request: RequestBuilder) -> Result<Value, SourceError> {
        let response = request.send().map_err(|_| SourceError::Connectivity)?;
        match response.status() {
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
        if response.status() == 204 || response.bytes().is_empty() {
            return Ok(Value::Null);
        }
        let value: Value = response.json().map_err(|_| malformed())?;
        if value.get("success") == Some(&Value::Bool(false)) || value.get("error").is_some() {
            return Err(SourceError::Backend("Clippsly rejected the request".into()));
        }
        Ok(value)
    }

    fn get(&self, path: &str) -> Result<Value, SourceError> {
        Self::response(self.request("GET", path)?)
    }

    pub fn account(&self) -> Result<Account, SourceError> {
        field(self.get("v1/account")?, "account")
    }

    pub fn storage(&self) -> Result<StorageQuota, SourceError> {
        decode(self.get("v1/account/storage")?)
    }

    pub fn tracks(&self) -> Result<Vec<Track>, SourceError> {
        let mut tracks = Vec::new();
        let mut seen = HashSet::new();
        loop {
            let response = self.get(&format!(
                "v1/library/tracks?limit={PAGE}&offset={}",
                tracks.len()
            ))?;
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
            if count < PAGE {
                return Ok(tracks);
            }
        }
    }

    pub fn stream(&self, id: &str) -> Result<Stream, SourceError> {
        let id = item_id(id)?;
        field(
            self.get(&format!("v1/tracks/{id}/stream?variant=best"))?,
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

    pub fn upload(&self, filename: &str, content: &[u8]) -> Result<(), SourceError> {
        validate_upload(filename, content.len())?;
        let quota = self.storage()?;
        if content.len() as u64 > quota.quota_remaining_bytes {
            return Err(SourceError::InvalidInput(
                "the file exceeds the remaining Clippsly storage quota".into(),
            ));
        }
        let (body, content_type) = multipart("audio", filename, content);
        Self::response(
            self.request("POST", "v1/upload")?
                .timeout(Duration::from_secs(600))
                .header("Content-Type", content_type)
                .body(body),
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client() -> ClippslyClient {
        ClippslyClient::new("https://listener.example.test", "test-app-password").unwrap()
    }

    #[test]
    fn a_direct_stream_url_wins_and_a_proxied_one_carries_its_token() {
        let client = client();
        let direct = Stream {
            url: "/proxy/42".into(),
            direct_url: Some("https://cdn.example/song?signature=abc".into()),
            token: None,
            bitrate: None,
            duration_seconds: None,
        };
        assert_eq!(
            client.stream_url(&direct).unwrap(),
            "https://cdn.example/song?signature=abc"
        );
        let proxied = Stream {
            url: "/proxy/42".into(),
            direct_url: None,
            token: Some("t0k".into()),
            bitrate: None,
            duration_seconds: None,
        };
        assert_eq!(
            client.stream_url(&proxied).unwrap(),
            "https://listener.example.test/proxy/42?token=t0k"
        );
    }

    #[test]
    fn media_urls_resolve_relative_and_refuse_other_schemes() {
        let client = client();
        assert!(
            client
                .media_url("/covers/42")
                .unwrap()
                .ends_with("/covers/42")
        );
        assert!(client.media_url("file:///etc/passwd").is_none());
        assert!(client.media_url("  ").is_none());
    }

    #[test]
    fn an_item_id_must_be_a_number() {
        assert!(item_id("42").is_ok());
        assert!(item_id("../account").is_err());
    }

    #[test]
    fn a_url_with_credentials_or_a_query_is_refused() {
        assert!(ClippslyClient::new("https://user:pw@listener.test", "p").is_err());
        assert!(ClippslyClient::new("https://listener.test/?a=b", "p").is_err());
        assert!(ClippslyClient::new("ftp://listener.test", "p").is_err());
    }

    #[test]
    fn clippsly_upload_validation_rejects_paths_empty_files_and_unsupported_formats() {
        assert!(validate_upload("song.FLAC", 1).is_ok());
        for (name, size) in [
            ("song.flac", 0),
            ("song.flac", MAX_UPLOAD_BYTES + 1),
            ("../song.flac", 1),
            ("song.txt", 1),
            ("song\n.flac", 1),
            ("so\"ng.flac", 1),
        ] {
            assert!(validate_upload(name, size).is_err(), "{name}");
        }
    }

    #[test]
    fn the_upload_body_is_one_file_part() {
        let (body, content_type) = multipart("audio", "song.flac", b"fLaC-test");
        let body = String::from_utf8(body).unwrap();
        let boundary = content_type
            .strip_prefix("multipart/form-data; boundary=")
            .unwrap();
        assert!(body.starts_with(&format!("--{boundary}\r\n")));
        assert!(body.contains("name=\"audio\"; filename=\"song.flac\""));
        assert!(body.contains("\r\n\r\nfLaC-test\r\n"));
        assert!(body.ends_with(&format!("--{boundary}--\r\n")));

        let (_, avoiding) = multipart("audio", "a.flac", b"--kopuz-clippsly-upload--");
        assert!(avoiding.ends_with("kopuz-clippsly-uploadx"));
    }

    #[test]
    fn broken_pages_do_not_decode() {
        for broken in [
            serde_json::json!({"success": true}),
            serde_json::json!({"tracks": [{}]}),
        ] {
            assert!(field::<Vec<Track>>(broken, "tracks").is_err());
        }
    }
}
