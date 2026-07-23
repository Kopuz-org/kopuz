//! Spotify OAuth (Authorization-Code + PKCE) with a loopback redirect.
//!
//! Unlike the YT/SoundCloud cookie-scrape flows, this is a real redirect flow:
//! we open the consent screen in the user's default browser, catch the redirect
//! on a localhost listener, and exchange the code (+ PKCE verifier) for tokens.
//! The Client ID is the user's own — each user registers an app at
//! developer.spotify.com and adds the redirect URI below.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};

/// Fixed loopback redirect. Spotify matches the registered URI byte-for-byte, so
/// the user must add exactly this on their app.
const REDIRECT_URI: &str = "http://127.0.0.1:8898/callback";
const REDIRECT_PORT: u16 = 8898;
const AUTH_URL: &str = "https://accounts.spotify.com/authorize";
const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";
/// `streaming` is required by the Web Playback SDK; the rest cover library reads
/// and issuing playback on the SDK device via the Connect API.
const SCOPES: &str = "streaming user-read-private user-library-read \
     user-library-modify playlist-read-private playlist-read-collaborative user-read-playback-state \
     user-modify-playback-state user-top-read user-read-recently-played";

/// Tokens (and, on initial sign-in, the account id) from a successful auth.
pub struct SpotifyAuth {
    pub access_token: String,
    pub refresh_token: String,
    pub user_id: String,
}

/// Pack `<access>\n<refresh>` into the single `access_token` config column so no
/// schema migration is needed for the extra refresh token.
pub fn pack_token(access: &str, refresh: &str) -> String {
    format!("{access}\n{refresh}")
}

/// Split a packed token back into `(access, refresh)`. A value with no newline is
/// treated as an access token with no refresh token.
pub fn unpack_token(packed: &str) -> (String, String) {
    match packed.split_once('\n') {
        Some((access, refresh)) => (access.to_string(), refresh.to_string()),
        None => (packed.to_string(), String::new()),
    }
}

/// Open the consent screen, catch the loopback redirect, and exchange the code
/// for tokens. `client_id` is the user's own Spotify app id.
pub async fn launch_signin_and_extract(client_id: String) -> Result<SpotifyAuth, String> {
    let client_id = client_id.trim().to_string();
    if client_id.is_empty() {
        return Err("Enter your Spotify app Client ID first".to_string());
    }

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

    let listener = std::net::TcpListener::bind(("127.0.0.1", REDIRECT_PORT)).map_err(|e| {
        format!("couldn't bind {REDIRECT_URI} (is it registered / is the port free?): {e}")
    })?;
    webbrowser::open(&auth_url).map_err(|e| format!("couldn't open the browser: {e}"))?;

    let expected_state = state.clone();
    let code = tokio::task::spawn_blocking(move || accept_code(listener, &expected_state))
        .await
        .map_err(|e| e.to_string())??;

    let redirect = REDIRECT_URI.to_string();
    let token = exchange(&[
        ("grant_type", "authorization_code"),
        ("code", &code),
        ("redirect_uri", &redirect),
        ("client_id", &client_id),
        ("code_verifier", &verifier),
    ])
    .await?;

    let user_id = fetch_user_id(&token.access_token).await;
    Ok(SpotifyAuth {
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        user_id,
    })
}

/// Refresh a packed `<access>\n<refresh>` pair and return the new pack, keeping
/// the old refresh token when Spotify doesn't rotate it. Shared by the periodic
/// refresh loop and the on-demand recovery when the SDK reports an auth error.
pub async fn refresh_packed(packed: &str, client_id: String) -> Result<String, String> {
    let (_access, refresh_tok) = unpack_token(packed);
    if refresh_tok.is_empty() {
        return Err("no Spotify refresh token stored".to_string());
    }
    let auth = refresh(refresh_tok.clone(), client_id).await?;
    let new_refresh = if auth.refresh_token.is_empty() {
        refresh_tok
    } else {
        auth.refresh_token
    };
    Ok(pack_token(&auth.access_token, &new_refresh))
}

/// Exchange a refresh token for a fresh access token. Spotify may or may not
/// return a new refresh token; when it doesn't, the caller keeps the old one.
pub async fn refresh(refresh_token: String, client_id: String) -> Result<SpotifyAuth, String> {
    let client_id = client_id.trim().to_string();
    if client_id.is_empty() {
        return Err("missing Spotify client id".to_string());
    }
    let token = exchange(&[
        ("grant_type", "refresh_token"),
        ("refresh_token", &refresh_token),
        ("client_id", &client_id),
    ])
    .await?;
    Ok(SpotifyAuth {
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        user_id: String::new(),
    })
}

#[derive(serde::Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: String,
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

async fn fetch_user_id(access: &str) -> String {
    let Ok(resp) = reqwest::Client::new()
        .get("https://api.spotify.com/v1/me")
        .bearer_auth(access)
        .send()
        .await
    else {
        return String::new();
    };
    resp.json::<serde_json::Value>()
        .await
        .ok()
        .and_then(|v| v.get("id").and_then(|id| id.as_str()).map(str::to_string))
        .unwrap_or_default()
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

/// Block until the browser hits `/callback`, validating `state` and returning the
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

    if !path.starts_with("/callback") {
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
    fn pack_unpack_roundtrip() {
        let packed = pack_token("acc", "ref");
        assert_eq!(
            unpack_token(&packed),
            ("acc".to_string(), "ref".to_string())
        );
    }

    #[test]
    fn unpack_legacy_bare_token() {
        assert_eq!(
            unpack_token("justaccess"),
            ("justaccess".to_string(), String::new())
        );
    }

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
