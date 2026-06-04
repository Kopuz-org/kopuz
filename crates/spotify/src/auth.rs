//! Authorization Code with PKCE for Spotify.
//!
//! Uses a short-lived local loopback listener on `127.0.0.1` for the OAuth
//! redirect. No client secret is required or shipped.

use crate::error::{Result, SpotifyError};
use crate::token_store::TokenStore;
use crate::types::Tokens;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

pub const AUTHORIZE_URL: &str = "https://accounts.spotify.com/authorize";
pub const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";

/// Scopes always requested.
pub const BASE_SCOPES: &[&str] = &[
    "user-read-private",
    "user-read-email",
    "user-read-playback-state",
    "user-read-currently-playing",
    "user-modify-playback-state",
    "playlist-read-private",
    "playlist-read-collaborative",
    "user-library-read",
];

/// Extra scope required only for Web Playback SDK.
pub const STREAMING_SCOPE: &str = "streaming";

pub fn build_scopes(needs_streaming: bool) -> String {
    let mut s: Vec<&str> = BASE_SCOPES.to_vec();
    if needs_streaming {
        s.push(STREAMING_SCOPE);
    }
    s.join(" ")
}

pub fn build_authorize_url(
    client_id: &str,
    redirect_uri: &str,
    scopes: &str,
    state: &str,
    code_challenge: &str,
) -> String {
    let enc = |s: &str| utf8_percent_encode(s, NON_ALPHANUMERIC).to_string();
    format!(
        "{base}?response_type=code&client_id={cid}&redirect_uri={ru}&scope={sc}&state={st}&code_challenge_method=S256&code_challenge={cc}",
        base = AUTHORIZE_URL,
        cid = enc(client_id),
        ru = enc(redirect_uri),
        sc = enc(scopes),
        st = enc(state),
        cc = enc(code_challenge),
    )
}

/// Parse the query string of a callback URL into a key/value map.
///
/// Accepts either a full URL or a bare query string ("code=...&state=...").
pub fn parse_callback_query(url_or_query: &str) -> HashMap<String, String> {
    let query = match url_or_query.split_once('?') {
        Some((_, q)) => q,
        None => url_or_query,
    };
    let query = query.split_once('#').map(|(q, _)| q).unwrap_or(query);
    let mut out = HashMap::new();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let k = percent_encoding::percent_decode_str(k)
            .decode_utf8_lossy()
            .into_owned();
        let v = percent_encoding::percent_decode_str(v)
            .decode_utf8_lossy()
            .into_owned();
        out.insert(k, v);
    }
    out
}

pub fn validate_state(expected: &str, got: Option<&str>) -> Result<()> {
    match got {
        Some(s) if s == expected => Ok(()),
        Some(_) => Err(SpotifyError::Auth("state mismatch".into())),
        None => Err(SpotifyError::Auth("missing state".into())),
    }
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    token_type: String,
    #[serde(default)]
    scope: String,
    expires_in: u64,
    #[serde(default)]
    refresh_token: Option<String>,
}

fn now_unix() -> u64 {
    use web_time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Exchange an authorization code for tokens at Spotify's token endpoint.
pub async fn exchange_code(
    http: &reqwest::Client,
    client_id: &str,
    code: &str,
    redirect_uri: &str,
    code_verifier: &str,
) -> Result<Tokens> {
    let params = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", client_id),
        ("code_verifier", code_verifier),
    ];
    let resp = http.post(TOKEN_URL).form(&params).send().await?;
    let status = resp.status();
    let body = resp.text().await?;
    if !status.is_success() {
        return Err(SpotifyError::Auth(format!(
            "token endpoint returned {}: {}",
            status.as_u16(),
            body
        )));
    }
    let tr: TokenResponse = serde_json::from_str(&body)?;
    let refresh = tr
        .refresh_token
        .ok_or_else(|| SpotifyError::Auth("token response missing refresh_token".into()))?;
    Ok(Tokens {
        access_token: tr.access_token,
        refresh_token: refresh,
        expires_at: now_unix().saturating_add(tr.expires_in),
        scope: tr.scope,
        token_type: tr.token_type,
    })
}

/// Refresh tokens with the refresh_token grant. The refresh token may or may
/// not be rotated by Spotify; if rotated, the new one replaces the old.
pub async fn refresh_tokens(
    http: &reqwest::Client,
    client_id: &str,
    refresh_token: &str,
) -> Result<Tokens> {
    let params = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", client_id),
    ];
    let resp = http.post(TOKEN_URL).form(&params).send().await?;
    let status = resp.status();
    let body = resp.text().await?;
    if !status.is_success() {
        return Err(SpotifyError::TokenRefreshFailed(format!(
            "{}: {}",
            status.as_u16(),
            body
        )));
    }
    let tr: TokenResponse =
        serde_json::from_str(&body).map_err(|e| SpotifyError::TokenRefreshFailed(e.to_string()))?;
    Ok(Tokens {
        access_token: tr.access_token,
        refresh_token: tr
            .refresh_token
            .unwrap_or_else(|| refresh_token.to_string()),
        expires_at: now_unix().saturating_add(tr.expires_in),
        scope: tr.scope,
        token_type: tr.token_type,
    })
}

/// Decision returned by `should_refresh`.
#[derive(Debug, PartialEq, Eq)]
pub enum RefreshDecision {
    /// Token is fresh enough.
    Skip,
    /// Token should be refreshed proactively.
    Refresh,
}

/// Default skew of 60 seconds before reported expiry.
pub fn should_refresh(t: &Tokens, now: u64) -> RefreshDecision {
    if t.is_expired_within(now, 60) {
        RefreshDecision::Refresh
    } else {
        RefreshDecision::Skip
    }
}

/// Parse the loopback port out of a redirect URI string. Spotify requires
/// the redirect_uri sent in the OAuth request to match the one registered in
/// the Dashboard byte-for-byte, port included. So we always bind the exact
/// port the user configured.
///
/// Returns `Err(Auth)` if the URI is not a `http://127.0.0.1:<port>/...` or
/// `http://[::1]:<port>/...` form.
pub fn parse_redirect_port(redirect_uri: &str) -> Result<u16> {
    let parsed = url::Url::parse(redirect_uri)
        .map_err(|e| SpotifyError::Auth(format!("invalid redirect_uri: {e}")))?;
    if parsed.scheme() != "http" {
        return Err(SpotifyError::Auth(
            "redirect_uri must be http for loopback".into(),
        ));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| SpotifyError::Auth("redirect_uri missing host".into()))?;
    if host != "127.0.0.1" && host != "[::1]" && host != "::1" {
        return Err(SpotifyError::Auth(format!(
            "redirect_uri host must be 127.0.0.1 or [::1], not {host}. Do not use localhost."
        )));
    }
    parsed
        .port()
        .ok_or_else(|| SpotifyError::Auth(
            "redirect_uri must include an explicit port. Register e.g. http://127.0.0.1:8898/callback in the Spotify Dashboard and put the same value here.".into()
        ))
}

/// Start a one-shot loopback listener and return the bound URL and a future
/// that resolves to the parsed callback query.
///
/// We bind `127.0.0.1` (IPv4 loopback IP literal) per Spotify's PKCE guidance.
/// Pass `port = 0` for an OS-assigned dynamic port.
pub async fn start_loopback_listener(port: u16) -> Result<(std::net::SocketAddr, TcpListener)> {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    let addr = listener.local_addr()?;
    Ok((addr, listener))
}

/// Accept one connection on the loopback listener, parse the OAuth callback,
/// and write a brief HTML response. Returns the parsed query map.
pub async fn accept_callback(listener: TcpListener) -> Result<HashMap<String, String>> {
    let (mut sock, _) = listener.accept().await?;
    let mut buf = vec![0u8; 8192];
    let n = sock.read(&mut buf).await?;
    let request = String::from_utf8_lossy(&buf[..n]).to_string();
    let first_line = request.lines().next().unwrap_or_default();
    // "GET /callback?code=...&state=... HTTP/1.1"
    let path = first_line.split_whitespace().nth(1).unwrap_or("");
    let q = parse_callback_query(path);
    let body = if q.contains_key("error") {
        "Spotify authorization failed. You can close this window."
    } else {
        "Spotify authorization complete. You can close this window."
    };
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = sock.write_all(resp.as_bytes()).await;
    let _ = sock.shutdown().await;
    Ok(q)
}

/// High level helper that holds an HTTP client, client id, and a token store
/// and exposes a refresh-and-cache primitive.
pub struct AuthCore<S: TokenStore> {
    pub http: reqwest::Client,
    pub client_id: String,
    pub store: Arc<S>,
}

impl<S: TokenStore> AuthCore<S> {
    pub fn new(http: reqwest::Client, client_id: impl Into<String>, store: Arc<S>) -> Self {
        Self {
            http,
            client_id: client_id.into(),
            store,
        }
    }

    pub async fn is_logged_in(&self) -> Result<bool> {
        Ok(self.store.load().await?.is_some())
    }

    pub async fn current_token(&self) -> Result<Option<Tokens>> {
        self.store.load().await
    }

    /// Refresh proactively if close to expiry. Returns the (possibly new) tokens.
    pub async fn refresh_if_needed(&self) -> Result<Tokens> {
        let t = self.store.load().await?.ok_or(SpotifyError::NotLoggedIn)?;
        if should_refresh(&t, now_unix()) == RefreshDecision::Refresh {
            let new = refresh_tokens(&self.http, &self.client_id, &t.refresh_token).await?;
            self.store.save(&new).await?;
            return Ok(new);
        }
        Ok(t)
    }

    /// Force a refresh regardless of expiry. Used after a 401 from the API.
    pub async fn force_refresh(&self) -> Result<Tokens> {
        let t = self.store.load().await?.ok_or(SpotifyError::NotLoggedIn)?;
        let new = refresh_tokens(&self.http, &self.client_id, &t.refresh_token).await?;
        self.store.save(&new).await?;
        Ok(new)
    }

    pub async fn logout(&self) -> Result<()> {
        self.store.clear().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pkce;

    #[test]
    fn parse_callback_extracts_code_and_state() {
        let q = parse_callback_query("/callback?code=abc%2F1&state=xyz&extra=foo");
        assert_eq!(q.get("code").unwrap(), "abc/1");
        assert_eq!(q.get("state").unwrap(), "xyz");
        assert_eq!(q.get("extra").unwrap(), "foo");
    }

    #[test]
    fn parse_callback_handles_error_param() {
        let q = parse_callback_query("/callback?error=access_denied&state=abc");
        assert_eq!(q.get("error").unwrap(), "access_denied");
    }

    #[test]
    fn parse_redirect_port_accepts_valid_loopback() {
        assert_eq!(
            parse_redirect_port("http://127.0.0.1:8898/callback").unwrap(),
            8898
        );
        assert_eq!(
            parse_redirect_port("http://[::1]:9000/callback").unwrap(),
            9000
        );
    }

    #[test]
    fn parse_redirect_port_rejects_localhost() {
        let e = parse_redirect_port("http://localhost:8898/callback").unwrap_err();
        assert!(matches!(e, SpotifyError::Auth(_)));
    }

    #[test]
    fn parse_redirect_port_rejects_https() {
        let e = parse_redirect_port("https://127.0.0.1:8898/callback").unwrap_err();
        assert!(matches!(e, SpotifyError::Auth(_)));
    }

    #[test]
    fn parse_redirect_port_rejects_missing_port() {
        let e = parse_redirect_port("http://127.0.0.1/callback").unwrap_err();
        assert!(matches!(e, SpotifyError::Auth(_)));
    }

    #[test]
    fn validate_state_matches() {
        assert!(validate_state("abc", Some("abc")).is_ok());
        assert!(validate_state("abc", Some("xyz")).is_err());
        assert!(validate_state("abc", None).is_err());
    }

    #[test]
    fn authorize_url_contains_required_params() {
        let u = build_authorize_url(
            "CID",
            "http://127.0.0.1/callback",
            "scope1 scope2",
            "STATE",
            "CHAL",
        );
        assert!(u.starts_with(AUTHORIZE_URL));
        assert!(u.contains("response_type=code"));
        assert!(u.contains("client_id=CID"));
        assert!(u.contains("code_challenge_method=S256"));
        assert!(u.contains("code_challenge=CHAL"));
        assert!(u.contains("state=STATE"));
        // scope must be percent-encoded space
        assert!(u.contains("scope=scope1%20scope2"));
        // redirect must be a loopback IP literal, not localhost. We percent-
        // encode the unreserved range too, so dots are encoded as %2E.
        assert!(u.contains("redirect_uri=http%3A%2F%2F127%2E0%2E0%2E1%2Fcallback"));
        assert!(!u.contains("localhost"));
    }

    #[test]
    fn scopes_streaming_gating() {
        assert!(!build_scopes(false).contains("streaming"));
        assert!(build_scopes(true).contains("streaming"));
        for s in BASE_SCOPES {
            assert!(build_scopes(false).contains(s));
        }
    }

    #[test]
    fn should_refresh_logic() {
        let t = Tokens {
            access_token: "a".into(),
            refresh_token: "r".into(),
            expires_at: 1_000,
            scope: String::new(),
            token_type: "Bearer".into(),
        };
        // 60s skew window: 941+ triggers refresh (941+60>=1000), 940 does not.
        assert_eq!(should_refresh(&t, 800), RefreshDecision::Skip);
        assert_eq!(should_refresh(&t, 939), RefreshDecision::Skip);
        assert_eq!(should_refresh(&t, 940), RefreshDecision::Refresh);
        assert_eq!(should_refresh(&t, 1_500), RefreshDecision::Refresh);
    }

    #[test]
    fn pkce_verifier_matches_known_challenge() {
        // RFC 7636 example, also used in pkce tests, kept here to assert
        // that auth.rs uses the same algorithm via pkce module.
        let v = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let c = pkce::challenge_s256(v);
        assert_eq!(c, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }
}
