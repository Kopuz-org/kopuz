//! Registered app configuration and bounded, loopback-only browser handoff.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

pub static APP_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct AppCredentials {
    pub client_id: String,
    pub client_secret: String,
    pub developer_token: String,
    pub soundcloud_token: Option<OAuthToken>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct OAuthToken {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: u64,
}

impl AppCredentials {
    pub async fn load(db: &db::Db, id: &str) -> Result<Self, String> {
        match db.browser_auth(id).await.map_err(|e| e.to_string())? {
            Some(json) => serde_json::from_str(&json)
                .map_err(|_| "Invalid stored browser app configuration".to_string()),
            None => Ok(Self::default()),
        }
    }

    pub async fn save(&self, db: &db::Db, id: &str) -> Result<(), String> {
        let json = serde_json::to_string(self).map_err(|e| e.to_string())?;
        db.set_browser_auth(id, &json)
            .await
            .map_err(|e| e.to_string())
    }
}

pub fn nonce() -> String {
    URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>())
}

pub fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub struct Loopback {
    listener: TcpListener,
    pub origin: String,
}

pub struct Request {
    stream: TcpStream,
    pub method: String,
    pub target: String,
    pub origin: Option<String>,
    pub body: Vec<u8>,
}

impl Loopback {
    pub async fn bind(port: u16) -> Result<Self, String> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
            .await
            .map_err(|_| {
                format!("Sign-in port {port} is busy. Finish the other sign-in and retry.")
            })?;
        Ok(Self {
            listener,
            origin: format!("http://127.0.0.1:{port}"),
        })
    }

    pub async fn request(&self) -> Result<Request, String> {
        loop {
            let (stream, _) = self.listener.accept().await.map_err(|e| e.to_string())?;
            // Browsers may preconnect without sending a request. Bound those
            // sockets as well as the overall authorization wait.
            if let Ok(Ok(request)) = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                Request::read(stream, &self.origin),
            )
            .await
            {
                return Ok(request);
            }
        }
    }
}

impl Request {
    async fn read(mut stream: TcpStream, origin: &str) -> Result<Self, String> {
        let mut bytes = Vec::new();
        let end = loop {
            let mut chunk = [0u8; 1024];
            let n = stream.read(&mut chunk).await.map_err(|e| e.to_string())?;
            if n == 0 || bytes.len() + n > 24_576 {
                return Err("Invalid sign-in request".into());
            }
            bytes.extend_from_slice(&chunk[..n]);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                break end + 4;
            }
            if bytes.len() > 8192 {
                return Err("Sign-in headers too large".into());
            }
        };
        let header = std::str::from_utf8(&bytes[..end]).map_err(|_| "Invalid headers")?;
        let mut lines = header.split("\r\n");
        let parts: Vec<_> = lines
            .next()
            .unwrap_or_default()
            .split_whitespace()
            .collect();
        if parts.len() != 3 || !parts[1].starts_with('/') {
            return Err("Invalid request line".into());
        }
        let method = parts[0].to_string();
        let target = parts[1].to_string();
        let mut length = None;
        let mut host = None;
        let mut request_origin = None;
        for line in lines.filter(|line| !line.is_empty()) {
            let (key, value) = line.split_once(':').ok_or("Invalid header")?;
            let value = value.trim();
            match key.to_ascii_lowercase().as_str() {
                "host" if host.is_none() => host = Some(value),
                "origin" if request_origin.is_none() => request_origin = Some(value.to_string()),
                "content-length" if length.is_none() => {
                    length = Some(value.parse::<usize>().map_err(|_| "Invalid length")?)
                }
                "host" | "origin" | "content-length" | "transfer-encoding" => {
                    return Err("Ambiguous request".into());
                }
                _ => {}
            }
        }
        if host != origin.strip_prefix("http://") {
            return Err("Invalid host".into());
        }
        let length = length.unwrap_or(0);
        if length > 16_384 {
            return Err("Sign-in body too large".into());
        }
        let mut body = bytes[end..].to_vec();
        if body.len() > length {
            return Err("Invalid body length".into());
        }
        let offset = body.len();
        body.resize(length, 0);
        stream
            .read_exact(&mut body[offset..])
            .await
            .map_err(|e| e.to_string())?;
        Ok(Self {
            stream,
            method,
            target,
            origin: request_origin,
            body,
        })
    }

    pub async fn respond(mut self, status: &str, content_type: &str, body: &str) {
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            self.stream.write_all(response.as_bytes()),
        )
        .await;
    }
}

pub fn authorization_code(target: &str, state: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(&format!("http://127.0.0.1{target}"))
        .map_err(|_| "Invalid authorization callback")?;
    if url.path() != "/callback" {
        return Err("Unexpected authorization callback".into());
    }
    let values: Vec<_> = url.query_pairs().collect();
    for key in ["state", "code", "error"] {
        if values.iter().filter(|(k, _)| k == key).count() > 1 {
            return Err("Ambiguous authorization callback".into());
        }
    }
    let value = |key| {
        values
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_ref())
    };
    if value("state") != Some(state) {
        return Err("Authorization state did not match".into());
    }
    if value("error").is_some() {
        return Err("Authorization was declined. Return to Kopuz and retry.".into());
    }
    value("code")
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "Missing authorization code".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_requires_exact_path_unique_state_and_code() {
        assert_eq!(
            authorization_code("/callback?state=expected&code=a%2Bb", "expected").unwrap(),
            "a+b"
        );
        for target in [
            "/callback-extra?state=expected&code=x",
            "/callback?state=wrong&code=x",
            "/callback?state=expected&state=expected&code=x",
            "/callback?state=expected&code=x&code=y",
            "/callback?state=expected&error=access_denied",
            "/callback?state=expected&code=",
        ] {
            assert!(authorization_code(target, "expected").is_err());
        }
    }

    #[test]
    fn pkce_matches_rfc_7636() {
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }
}
