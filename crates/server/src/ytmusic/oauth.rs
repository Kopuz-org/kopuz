//! User-approved OAuth device authorization for registered YouTube clients.

use reqwest::RequestBuilder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::Duration;

use crate::browser_auth::{self, APP_LOCK, AppCredentials, Loopback, OAuthToken};

pub const SESSION_MARKER: &str = "kopuz:youtube:oauth:v1";
const BEARER_PREFIX: &str = "kopuz:youtube:bearer:v1:";
const DEVICE_URL: &str = "https://oauth2.googleapis.com/device/code";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

#[derive(Deserialize)]
struct DeviceCode {
    device_code: String,
    user_code: String,
    #[serde(alias = "verification_uri")]
    verification_url: String,
    expires_in: u64,
    #[serde(default = "default_interval")]
    interval: u64,
}

fn default_interval() -> u64 {
    5
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    token_type: String,
    expires_in: u64,
}

impl TokenResponse {
    fn into_token(self, previous_refresh: Option<&str>) -> Result<OAuthToken, String> {
        let refresh_token = if self.refresh_token.is_empty() {
            previous_refresh.unwrap_or_default().to_string()
        } else {
            self.refresh_token
        };
        if self.access_token.is_empty()
            || refresh_token.is_empty()
            || self.expires_in == 0
            || !self.token_type.eq_ignore_ascii_case("bearer")
        {
            return Err("Google returned incomplete OAuth credentials".into());
        }
        Ok(OAuthToken {
            access_token: self.access_token,
            refresh_token,
            expires_at: browser_auth::now().saturating_add(self.expires_in),
        })
    }
}

fn http() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())
}

async fn response_json(response: reqwest::Response) -> Result<serde_json::Value, String> {
    response
        .json()
        .await
        .map_err(|_| "Google returned an invalid authorization response".into())
}

fn verification_url(value: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(value).map_err(|_| "Invalid Google verification URL")?;
    if url.scheme() != "https"
        || !matches!(
            url.host_str(),
            Some("www.google.com" | "accounts.google.com")
        )
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port_or_known_default() != Some(443)
    {
        return Err("Unexpected Google verification URL".into());
    }
    Ok(url.to_string())
}

pub async fn sign_in(
    app: &AppCredentials,
    open: impl FnOnce(&str) -> Result<(), String>,
) -> Result<(OAuthToken, String), String> {
    if app.client_id.trim().is_empty() || app.client_secret.trim().is_empty() {
        return Err("Enter your registered Google OAuth Client ID and Client Secret first".into());
    }
    let listener = Loopback::bind(8897).await?;
    let http = http()?;
    let response = http
        .post(DEVICE_URL)
        .form(&[
            ("client_id", app.client_id.as_str()),
            ("scope", "https://www.googleapis.com/auth/youtube"),
        ])
        .send()
        .await
        .map_err(|_| "Could not reach Google authorization")?;
    if !response.status().is_success() {
        return Err(format!(
            "Google could not start device authorization ({}). Check the registered client type and YouTube Data API access.",
            response.status()
        ));
    }
    let code: DeviceCode = response
        .json()
        .await
        .map_err(|_| "Google returned an invalid device code")?;
    if code.device_code.is_empty() || code.user_code.is_empty() || code.expires_in == 0 {
        return Err("Google returned an incomplete device code".into());
    }
    let verification = verification_url(&code.verification_url)?;
    let state = browser_auth::nonce();
    let path = format!("/sign-in/{state}");
    let config = serde_json::to_string(
        &serde_json::json!({"code":code.user_code,"url":verification,"state":state}),
    )
    .map_err(|e| e.to_string())?
    .replace('<', "\\u003c");
    let page = include_str!("oauth.html").replace("__KOPUZ_CONFIGURATION__", &config);
    open(&format!("{}{path}", listener.origin))?;
    let token = tokio::time::timeout(Duration::from_secs(code.expires_in.min(1800)), async {
        let poll = poll_token(&http, app, &code);
        tokio::pin!(poll);
        loop {
            tokio::select! {
                token = &mut poll => return token,
                request = listener.request() => {
                    let request = request?;
                    if request.method == "GET" && request.target == path {
                        request.respond("200 OK", "text/html; charset=utf-8", &page).await;
                    } else if request.method == "POST" && request.target == "/cancel"
                        && request.origin.as_deref() == Some(listener.origin.as_str())
                        && serde_json::from_slice::<serde_json::Value>(&request.body).ok().and_then(|v| v["state"].as_str().map(str::to_string)).as_deref() == Some(state.as_str()) {
                        request.respond("200 OK", "text/plain", "Cancelled").await;
                        return Err("YouTube Music sign-in was cancelled".into());
                    } else {
                        request.respond("404 Not Found", "text/plain", "Not found").await;
                    }
                }
            }
        }
    }).await.map_err(|_| "Google authorization expired. Return to Kopuz and start sign-in again.")??;
    // A stable opaque cache key, independent of rotating access tokens. No
    // additional profile/email scope is needed to isolate this account's data.
    let user_id = format!(
        "yt-oauth-{}",
        hex::encode(&Sha256::digest(token.refresh_token.as_bytes())[..16])
    );
    let authorization = encode(&token.access_token, &user_id)?;
    super::YouTubeMusicClient::with_cookies(authorization).validate_cookies().await
        .map_err(|_| "Google authorized Kopuz, but YouTube Music did not accept the session. The account has not been saved as connected.")?;
    Ok((token, user_id))
}

async fn poll_token(
    http: &reqwest::Client,
    app: &AppCredentials,
    code: &DeviceCode,
) -> Result<OAuthToken, String> {
    let mut interval = code.interval.max(5);
    loop {
        tokio::time::sleep(Duration::from_secs(interval)).await;
        let response = http
            .post(TOKEN_URL)
            .form(&[
                ("client_id", app.client_id.as_str()),
                ("client_secret", app.client_secret.as_str()),
                ("device_code", code.device_code.as_str()),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ])
            .send()
            .await
            .map_err(|_| "Google authorization polling failed")?;
        let status = response.status();
        let value = response_json(response).await?;
        if status.is_success() {
            return serde_json::from_value::<TokenResponse>(value)
                .map_err(|_| "Invalid Google token response")?
                .into_token(None);
        }
        match value["error"].as_str() {
            Some("authorization_pending") => {}
            Some("slow_down") => interval = interval.saturating_add(5),
            Some("access_denied") => return Err("Google authorization was declined".into()),
            Some("expired_token") => {
                return Err("Google authorization code expired. Start sign-in again.".into());
            }
            _ => {
                return Err(format!(
                    "Google authorization failed ({status}). Check your OAuth client configuration."
                ));
            }
        }
    }
}

#[derive(Deserialize, Serialize)]
pub(super) struct Bearer {
    pub access_token: String,
    pub user_id: String,
}

pub(super) fn decode(value: &str) -> Option<Bearer> {
    serde_json::from_str(value.strip_prefix(BEARER_PREFIX)?).ok()
}

fn encode(access: &str, user: &str) -> Result<String, String> {
    serde_json::to_string(&Bearer {
        access_token: access.into(),
        user_id: user.into(),
    })
    .map(|value| format!("{BEARER_PREFIX}{value}"))
    .map_err(|e| e.to_string())
}

pub(super) fn authenticate(
    request: RequestBuilder,
    credentials: &str,
    origin: &str,
) -> Result<RequestBuilder, String> {
    if credentials.starts_with(BEARER_PREFIX) {
        let bearer = decode(credentials).ok_or("Invalid YouTube OAuth credentials")?;
        if bearer.access_token.is_empty() {
            return Err("Missing YouTube access token".into());
        }
        Ok(request
            .bearer_auth(bearer.access_token)
            .header("X-Goog-Request-Time", browser_auth::now().to_string()))
    } else {
        let authorization =
            super::innertube::sapisid_hash(credentials, origin).ok_or("SAPISID missing")?;
        Ok(request
            .header("Cookie", credentials)
            .header("Authorization", authorization))
    }
}

pub(super) struct Client {
    db: db::Db,
    id: String,
}

impl Client {
    pub fn new(db: db::Db, id: String) -> Self {
        Self { db, id }
    }

    pub async fn credentials(&self) -> Result<String, String> {
        let _guard = APP_LOCK.lock().await;
        if self
            .db
            .load_server(&self.id)
            .await
            .map_err(|e| e.to_string())?
            .is_none_or(|server| server.access_token.as_deref() != Some(SESSION_MARKER))
        {
            return Err("YouTube Music is signed out".into());
        }
        let mut app = AppCredentials::load(&self.db, &self.id).await?;
        let token = app
            .youtube_token
            .as_ref()
            .ok_or("Sign in to YouTube Music first")?;
        if token.expires_at <= browser_auth::now().saturating_add(120) {
            let response = http()?
                .post(TOKEN_URL)
                .form(&[
                    ("client_id", app.client_id.as_str()),
                    ("client_secret", app.client_secret.as_str()),
                    ("refresh_token", token.refresh_token.as_str()),
                    ("grant_type", "refresh_token"),
                ])
                .send()
                .await
                .map_err(|_| "Google token refresh failed")?;
            if !response.status().is_success() {
                return Err("Google could not renew authorization. Sign in again.".into());
            }
            let fresh = response
                .json::<TokenResponse>()
                .await
                .map_err(|_| "Invalid Google refresh response")?
                .into_token(Some(&token.refresh_token))?;
            app.youtube_token = Some(fresh);
            app.save(&self.db, &self.id).await?;
        }
        let token = app
            .youtube_token
            .as_ref()
            .ok_or("Missing Google access token")?;
        encode(&token.access_token, &app.youtube_user_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oauth_headers_do_not_put_tokens_in_cookies() {
        let value = encode("granted-access", "account-key").unwrap();
        let request = authenticate(
            reqwest::Client::new().get("https://music.youtube.com"),
            &value,
            "https://music.youtube.com",
        )
        .unwrap()
        .build()
        .unwrap();
        assert_eq!(request.headers()["Authorization"], "Bearer granted-access");
        assert!(!request.headers().contains_key("Cookie"));
        assert!(request.headers().contains_key("X-Goog-Request-Time"));
        assert_eq!(
            super::super::derive_user_id(&value).as_deref(),
            Some("account-key")
        );
    }

    #[test]
    fn cookie_sessions_keep_their_existing_authentication() {
        let request = authenticate(
            reqwest::Client::new().get("https://music.youtube.com"),
            "SAPISID=session",
            "https://music.youtube.com",
        )
        .unwrap()
        .build()
        .unwrap();
        assert_eq!(request.headers()["Cookie"], "SAPISID=session");
        assert!(
            request.headers()["Authorization"]
                .to_str()
                .unwrap()
                .starts_with("SAPISIDHASH ")
        );
        assert!(
            authenticate(
                reqwest::Client::new().get("https://music.youtube.com"),
                &format!("{BEARER_PREFIX}invalid"),
                "https://music.youtube.com"
            )
            .is_err()
        );
    }

    #[test]
    fn refresh_keeps_the_original_refresh_token_when_google_omits_it() {
        let token = TokenResponse {
            access_token: "new-access".into(),
            refresh_token: String::new(),
            token_type: "Bearer".into(),
            expires_in: 3600,
        }
        .into_token(Some("original-refresh"))
        .unwrap();
        assert_eq!(token.refresh_token, "original-refresh");
        assert!(token.expires_at > browser_auth::now());
    }

    #[test]
    fn only_google_verification_pages_are_opened() {
        assert!(verification_url("https://www.google.com/device").is_ok());
        for url in [
            "http://www.google.com/device",
            "https://example.org/device",
            "https://www.google.com:444/device",
            "https://user@www.google.com/device",
        ] {
            assert!(verification_url(url).is_err());
        }
    }
}
