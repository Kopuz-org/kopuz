//! A blocking HTTP client over the host's `http` import, shaped like reqwest
//! without the `.await`.

use std::time::Duration;

use base64::Engine as _;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::bindings::kopuz::plugin::http as wit;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Denied(String),
    Connect(String),
    Timeout,
    Request(String),
    Decode(String),
    Other(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Denied(message) => write!(f, "request refused: {message}"),
            Error::Connect(message) => write!(f, "could not connect: {message}"),
            Error::Timeout => f.write_str("the request timed out"),
            Error::Request(message) => write!(f, "invalid request: {message}"),
            Error::Decode(message) => write!(f, "unexpected response: {message}"),
            Error::Other(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for Error {}

impl Error {
    pub fn is_connectivity(&self) -> bool {
        matches!(self, Error::Connect(_) | Error::Timeout)
    }
}

impl From<wit::HttpError> for Error {
    fn from(error: wit::HttpError) -> Self {
        match error {
            wit::HttpError::Denied(message) => Error::Denied(message),
            wit::HttpError::Connect(message) => Error::Connect(message),
            wit::HttpError::Timeout => Error::Timeout,
            wit::HttpError::Other(message) => Error::Other(message),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Client {
    headers: Vec<(String, String)>,
    timeout: Option<Duration>,
}

impl Client {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn request(&self, method: &str, url: impl AsRef<str>) -> RequestBuilder {
        RequestBuilder {
            method: method.to_string(),
            url: url::Url::parse(url.as_ref())
                .map_err(|error| Error::Request(format!("invalid URL {}: {error}", url.as_ref()))),
            headers: self.headers.clone(),
            body: None,
            timeout: self.timeout,
        }
    }

    pub fn get(&self, url: impl AsRef<str>) -> RequestBuilder {
        self.request("GET", url)
    }

    pub fn post(&self, url: impl AsRef<str>) -> RequestBuilder {
        self.request("POST", url)
    }

    pub fn put(&self, url: impl AsRef<str>) -> RequestBuilder {
        self.request("PUT", url)
    }

    pub fn delete(&self, url: impl AsRef<str>) -> RequestBuilder {
        self.request("DELETE", url)
    }
}

#[derive(Debug, Clone)]
pub struct RequestBuilder {
    method: String,
    url: Result<url::Url, Error>,
    headers: Vec<(String, String)>,
    body: Option<Vec<u8>>,
    timeout: Option<Duration>,
}

impl RequestBuilder {
    pub fn query<K: AsRef<str>, V: AsRef<str>>(mut self, pairs: &[(K, V)]) -> Self {
        if let Ok(url) = &mut self.url {
            let mut query = url.query_pairs_mut();
            for (key, value) in pairs {
                query.append_pair(key.as_ref(), value.as_ref());
            }
        }
        self
    }

    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    pub fn bearer_auth(self, token: impl std::fmt::Display) -> Self {
        self.header("Authorization", format!("Bearer {token}"))
    }

    pub fn basic_auth(self, username: &str, password: Option<&str>) -> Self {
        let credentials = format!("{username}:{}", password.unwrap_or_default());
        let encoded = base64::engine::general_purpose::STANDARD.encode(credentials);
        self.header("Authorization", format!("Basic {encoded}"))
    }

    pub fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = Some(body.into());
        self
    }

    pub fn json<T: Serialize + ?Sized>(mut self, value: &T) -> Self {
        match serde_json::to_vec(value) {
            Ok(body) => {
                self.body = Some(body);
                self.header("Content-Type", "application/json")
            }
            Err(error) => {
                self.url = Err(Error::Request(format!("body is not JSON: {error}")));
                self
            }
        }
    }

    pub fn form<K: AsRef<str>, V: AsRef<str>>(mut self, pairs: &[(K, V)]) -> Self {
        let body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(pairs.iter().map(|(k, v)| (k.as_ref(), v.as_ref())))
            .finish();
        self.body = Some(body.into_bytes());
        self.header("Content-Type", "application/x-www-form-urlencoded")
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn url(&self) -> Result<&url::Url, &Error> {
        self.url.as_ref()
    }

    pub fn send(self) -> Result<Response, Error> {
        let url = self.url?;
        let response = wit::send(&wit::Request {
            method: self.method,
            url: url.into(),
            headers: self.headers,
            body: self.body,
            timeout_ms: self
                .timeout
                .map(|timeout| timeout.as_millis().min(u32::MAX as u128) as u32),
        })?;
        Ok(Response {
            status: response.status,
            headers: response.headers,
            body: response.body,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Response {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Response {
    pub fn status(&self) -> u16 {
        self.status
    }

    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub fn json<T: DeserializeOwned>(&self) -> Result<T, Error> {
        serde_json::from_slice(&self.body).map_err(|error| Error::Decode(error.to_string()))
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn bytes(&self) -> &[u8] {
        &self.body
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.body
    }
}
