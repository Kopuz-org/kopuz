//! Spotify sign-in: Authorization-Code + PKCE with a loopback redirect, as
//! the desktop client does it, then a librespot login that trades the
//! short-lived OAuth token for credentials that do not expire.
//!
//! The client id is the desktop client's own, so there is no app for the
//! user to register and no Development-Mode allowlist. The OAuth token is
//! used exactly once: the access point answers the login with a reusable
//! credential blob, and that blob is what the config stores.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use librespot_core::SessionConfig;
use librespot_core::authentication::Credentials;
use sha2::{Digest, Sha256};

/// Loopback redirect. The desktop client id accepts any loopback port at
/// this path, which is what lets librespot's `--oauth-port` be a choice.
const REDIRECT_URI: &str = "http://127.0.0.1:8898/login";
const REDIRECT_PORT: u16 = 8898;
const AUTH_URL: &str = "https://accounts.spotify.com/authorize";
const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";
/// What librespot asks for: everything the desktop client may want, since
/// the token is traded for a session rather than used against one endpoint.
const SCOPES: &str = "app-remote-control playlist-modify playlist-modify-private \
     playlist-modify-public playlist-read playlist-read-collaborative playlist-read-private \
     streaming ugc-image-upload user-follow-modify user-follow-read user-library-modify \
     user-library-read user-modify user-modify-playback-state user-modify-private \
     user-personalized user-read-birthdate user-read-currently-playing user-read-email \
     user-read-play-history user-read-playback-position user-read-playback-state \
     user-read-private user-read-recently-played user-top-read";

/// What a successful sign-in stores: the packed reusable credentials and
/// the account they belong to.
pub struct SpotifyAuth {
    pub stored: String,
    pub username: String,
}

/// Open the consent screen, catch the loopback redirect, exchange the code
/// for a token, and log a session in with it. `device_id` is kopuz's own, so
/// the account sees one device however often it signs in.
pub async fn sign_in(device_id: String) -> Result<SpotifyAuth, String> {
    let access = oauth_access_token().await?;
    let session = super::session::connect(Credentials::with_access_token(access), &device_id)
        .await
        .map_err(|error| format!("Spotify refused the login: {error}"))?;
    let credentials = super::session::reusable_credentials(&session);
    let stored = super::session::pack_credentials(&credentials).map_err(|e| e.to_string())?;
    // The session that just logged in is the one playback should use, so
    // the first track does not wait on a second handshake.
    super::session::shared(&stored, &device_id)
        .adopt(session)
        .await;
    Ok(SpotifyAuth {
        username: credentials.username.unwrap_or_default(),
        stored,
    })
}

/// The PKCE dance up to an access token.
async fn oauth_access_token() -> Result<String, String> {
    let client_id = SessionConfig::default().client_id;
    let verifier = gen_verifier();
    let challenge = code_challenge(&verifier);
    let state = gen_state();

    let auth_url = format!(
        "{AUTH_URL}?client_id={cid}&response_type=code&redirect_uri={redir}\
         &code_challenge_method=S256&code_challenge={challenge}&scope={scope}&state={state}",
        cid = urlencode(&client_id),
        redir = urlencode(REDIRECT_URI),
        scope = urlencode(SCOPES),
    );

    let listener = std::net::TcpListener::bind(("127.0.0.1", REDIRECT_PORT))
        .map_err(|e| format!("couldn't bind {REDIRECT_URI} (is the port free?): {e}"))?;
    webbrowser::open(&auth_url).map_err(|e| format!("couldn't open the browser: {e}"))?;

    let expected_state = state.clone();
    let code = tokio::task::spawn_blocking(move || accept_code(listener, &expected_state))
        .await
        .map_err(|e| e.to_string())??;

    let token = exchange(&[
        ("grant_type", "authorization_code"),
        ("code", &code),
        ("redirect_uri", REDIRECT_URI),
        ("client_id", &client_id),
        ("code_verifier", &verifier),
    ])
    .await?;
    Ok(token.access_token)
}

#[derive(serde::Deserialize)]
struct TokenResponse {
    access_token: String,
}

async fn exchange(params: &[(&str, &str)]) -> Result<TokenResponse, String> {
    let resp = reqwest::Client::new()
        .post(TOKEN_URL)
        .form(params)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Spotify token endpoint returned {status}: {body}"));
    }
    resp.json::<TokenResponse>()
        .await
        .map_err(|e| format!("couldn't parse Spotify token response: {e}"))
}

/// PKCE verifier: 64 random bytes → base64url (86 chars, within the 43..=128
/// range the spec allows).
fn gen_verifier() -> String {
    let bytes: [u8; 64] = rand::random();
    URL_SAFE_NO_PAD.encode(bytes)
}

fn code_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

fn gen_state() -> String {
    let bytes: [u8; 16] = rand::random();
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Block until the browser hits `/login`, validating `state` and returning the
/// authorization code. Times out so a cancelled sign-in doesn't leak the thread.
fn accept_code(listener: std::net::TcpListener, expected_state: &str) -> Result<String, String> {
    listener.set_nonblocking(true).ok();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
    loop {
        if std::time::Instant::now() > deadline {
            return Err("timed out waiting for Spotify sign-in".to_string());
        }
        match listener.accept() {
            Ok((stream, _)) => match handle_conn(stream, expected_state) {
                Ok(None) => continue,
                other => return other.map(|o| o.unwrap_or_default()),
            },
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

fn handle_conn(
    mut stream: std::net::TcpStream,
    expected_state: &str,
) -> Result<Option<String>, String> {
    use std::io::{Read, Write};

    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .ok();
    let mut buf = [0u8; 4096];
    let n = stream.read(&mut buf).unwrap_or(0);
    let req = String::from_utf8_lossy(&buf[..n]);
    let path = req
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("");

    if !path.starts_with("/login") {
        let _ = stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return Ok(None);
    }

    let query = path.split_once('?').map(|(_, q)| q).unwrap_or("");
    let (mut code, mut state, mut error) = (None, None, None);
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key {
            "code" => code = Some(urldecode(value)),
            "state" => state = Some(urldecode(value)),
            "error" => error = Some(urldecode(value)),
            _ => {}
        }
    }

    let body = "<!doctype html><html><body style=\"font-family:sans-serif;background:#121212;\
        color:#fff;text-align:center;padding-top:4rem\"><h2>kopuz is connected to Spotify.</h2>\
        <p>You can close this tab and return to the app.</p></body></html>";
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(resp.as_bytes());

    if let Some(err) = error {
        return Err(format!("Spotify authorization was denied: {err}"));
    }
    if state.as_deref() != Some(expected_state) {
        return Err("Spotify sign-in failed a security check (state mismatch)".to_string());
    }
    match code {
        Some(c) if !c.is_empty() => Ok(Some(c)),
        _ => Err("Spotify redirect carried no authorization code".to_string()),
    }
}

/// Percent-encode per RFC 3986 unreserved set — enough for query values here.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn urldecode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                if let (Some(hi), Some(lo)) = (hi, lo) {
                    out.push((hi * 16 + lo) as u8);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_is_url_safe_and_unpadded() {
        let c = code_challenge("test-verifier");
        assert!(!c.contains('='));
        assert!(!c.contains('+'));
        assert!(!c.contains('/'));
    }

    #[test]
    fn urlencode_reserved() {
        assert_eq!(urlencode("a b/c"), "a%20b%2Fc");
    }

    #[test]
    fn urldecode_roundtrip() {
        assert_eq!(urldecode("a%20b%2Fc"), "a b/c");
    }
}
