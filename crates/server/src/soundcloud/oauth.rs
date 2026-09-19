//! SoundCloud's registered OAuth client and public API. Web cookies and web
//! player client IDs are deliberately not involved in this path.

use crate::browser_auth::{self, AppCredentials, OAuthToken};
use serde_json::Value;

pub const SESSION_MARKER: &str = "kopuz:soundcloud:oauth:v1";
pub const REDIRECT_URI: &str = "http://127.0.0.1:8899/callback";
const API: &str = "https://api.soundcloud.com";
const TOKEN_URL: &str = "https://secure.soundcloud.com/oauth/token";

pub async fn sign_in(
    app: &AppCredentials,
    open_browser: impl FnOnce(&str) -> Result<(), String>,
) -> Result<(OAuthToken, String), String> {
    if app.client_id.is_empty() || app.client_secret.is_empty() {
        return Err("Enter your registered SoundCloud Client ID and Client Secret first".into());
    }
    let listener = browser_auth::Loopback::bind(8899).await?;
    let verifier = browser_auth::nonce();
    let state = browser_auth::nonce();
    let mut url = reqwest::Url::parse("https://secure.soundcloud.com/authorize")
        .map_err(|e| e.to_string())?;
    url.query_pairs_mut().extend_pairs(&[
        ("client_id", app.client_id.as_str()),
        ("redirect_uri", REDIRECT_URI),
        ("response_type", "code"),
        ("code_challenge", &browser_auth::challenge(&verifier)),
        ("code_challenge_method", "S256"),
        ("state", &state),
        ("display", "popup"),
    ]);
    open_browser(url.as_str())?;
    let code = tokio::time::timeout(std::time::Duration::from_secs(300), async {
        loop {
            let request = listener.request().await?;
            if request.method != "GET" || !request.target.starts_with("/callback?") {
                request
                    .respond("404 Not Found", "text/plain", "Not found")
                    .await;
                continue;
            }
            let result = browser_auth::authorization_code(&request.target, &state);
            let body = if result.is_ok() {
                "Authorization received. Return to Kopuz to finish signing in."
            } else {
                "Authorization failed. Return to Kopuz and retry."
            };
            request
                .respond(
                    if result.is_ok() {
                        "200 OK"
                    } else {
                        "400 Bad Request"
                    },
                    "text/plain; charset=utf-8",
                    body,
                )
                .await;
            return result;
        }
    })
    .await
    .map_err(|_| "SoundCloud sign-in timed out. Return to Kopuz and retry.")??;
    let token = exchange(
        app,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", REDIRECT_URI),
            ("code_verifier", &verifier),
        ],
    )
    .await?;
    let me = request(reqwest::Method::GET, "/me", &token.access_token).await?;
    let user = resource_id(&me).ok_or("SoundCloud returned no account ID")?;
    Ok((token, user))
}

async fn exchange(app: &AppCredentials, parameters: &[(&str, &str)]) -> Result<OAuthToken, String> {
    #[derive(serde::Deserialize)]
    struct Response {
        access_token: String,
        refresh_token: String,
        expires_in: u64,
    }
    let mut parameters = parameters.to_vec();
    parameters.extend([
        ("client_id", app.client_id.as_str()),
        ("client_secret", app.client_secret.as_str()),
    ]);
    let response = reqwest::Client::new()
        .post(TOKEN_URL)
        .form(&parameters)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|_| "SoundCloud token request failed")?;
    if !response.status().is_success() {
        return Err(format!(
            "SoundCloud token request failed ({})",
            response.status()
        ));
    }
    let response: Response = response
        .json()
        .await
        .map_err(|_| "Invalid SoundCloud token response")?;
    if response.access_token.is_empty()
        || response.refresh_token.is_empty()
        || response.expires_in == 0
    {
        return Err("SoundCloud returned an incomplete token".into());
    }
    Ok(OAuthToken {
        access_token: response.access_token,
        refresh_token: response.refresh_token,
        expires_at: browser_auth::now().saturating_add(response.expires_in),
    })
}

pub(crate) fn resource_id(value: &Value) -> Option<String> {
    value
        .get("urn")
        .and_then(Value::as_str)
        .and_then(|urn| urn.rsplit(':').next())
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .or_else(|| {
            value
                .get("id")
                .and_then(Value::as_u64)
                .map(|id| id.to_string())
        })
}

fn api_url(path: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(API)
        .and_then(|base| base.join(path))
        .map_err(|_| "Invalid SoundCloud API URL")?;
    if url.scheme() != "https"
        || url.host_str() != Some("api.soundcloud.com")
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Unexpected SoundCloud API origin".into());
    }
    Ok(url)
}

async fn request(method: reqwest::Method, path: &str, token: &str) -> Result<Value, String> {
    let response = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?
        .request(method, api_url(path)?)
        .header("Authorization", format!("OAuth {token}"))
        .header("Accept", "application/json")
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|_| "SoundCloud API request failed")?;
    if !response.status().is_success() {
        return Err(format!("SoundCloud API returned {}", response.status()));
    }
    if response.status() == reqwest::StatusCode::NO_CONTENT {
        return Ok(Value::Null);
    }
    response
        .json()
        .await
        .map_err(|_| "Invalid SoundCloud API response".into())
}

pub(crate) struct Client {
    db: db::Db,
    server_id: String,
}

impl Client {
    pub fn new(db: db::Db, server_id: String) -> Self {
        Self { db, server_id }
    }

    async fn token(&self) -> Result<String, String> {
        // Refresh tokens are single-use. Serialize refresh with configuration
        // changes and persist the replacement before releasing this lock.
        let _guard = browser_auth::APP_LOCK.lock().await;
        let server = self
            .db
            .load_server(&self.server_id)
            .await
            .map_err(|e| e.to_string())?;
        if server.is_none_or(|s| s.access_token.as_deref() != Some(SESSION_MARKER)) {
            return Err("SoundCloud is signed out".into());
        }
        let mut app = AppCredentials::load(&self.db, &self.server_id).await?;
        let token = app
            .soundcloud_token
            .as_ref()
            .ok_or("Sign in to SoundCloud first")?;
        if token.expires_at > browser_auth::now().saturating_add(120) {
            return Ok(token.access_token.clone());
        }
        let fresh = exchange(
            &app,
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", &token.refresh_token),
            ],
        )
        .await?;
        let access = fresh.access_token.clone();
        app.soundcloud_token = Some(fresh);
        app.save(&self.db, &self.server_id).await?;
        Ok(access)
    }

    async fn get(&self, path: &str) -> Result<Value, String> {
        request(reqwest::Method::GET, path, &self.token().await?).await
    }

    pub async fn validate(&self) -> Result<(), String> {
        self.get("/me").await.map(|_| ())
    }

    pub async fn search(&self, query: &str) -> Result<Vec<reader::Track>, String> {
        let mut url = api_url("/tracks")?;
        url.query_pairs_mut().extend_pairs(&[
            ("q", query),
            ("limit", "50"),
            ("linked_partitioning", "true"),
        ]);
        Ok(tracks(&self.get(url.as_str()).await?))
    }

    pub async fn favorites(
        &self,
        cursor: Option<&str>,
    ) -> Result<(Vec<reader::Track>, Option<String>), String> {
        let value = self
            .get(cursor.unwrap_or("/me/likes/tracks?limit=200&linked_partitioning=true"))
            .await?;
        Ok((tracks(&value), next(&value)))
    }

    pub async fn playlists(&self) -> Result<Vec<super::PlaylistSummary>, String> {
        let mut path =
            Some("/me/playlists?limit=200&linked_partitioning=true&show_tracks=false".to_string());
        let mut result = Vec::new();
        let mut visited = std::collections::HashSet::new();
        while let Some(url) = path {
            if !visited.insert(url.clone()) {
                return Err("SoundCloud repeated a page".into());
            }
            let value = self.get(&url).await?;
            for item in collection(&value) {
                if let Some(id) = resource_id(item) {
                    result.push(super::PlaylistSummary {
                        id,
                        title: item["title"].as_str().unwrap_or_default().to_string(),
                        artwork_url: item["artwork_url"].as_str().map(str::to_string),
                    });
                }
            }
            path = next(&value);
        }
        Ok(result)
    }

    pub async fn playlist_tracks(&self, id: &str) -> Result<Vec<reader::Track>, String> {
        let mut path = Some(format!(
            "/playlists/{}/tracks?limit=200&linked_partitioning=true",
            resource_path("playlists", id)?
        ));
        let mut result = Vec::new();
        let mut visited = std::collections::HashSet::new();
        while let Some(url) = path {
            if !visited.insert(url.clone()) {
                return Err("SoundCloud repeated a page".into());
            }
            let value = self.get(&url).await?;
            result.extend(tracks(&value));
            path = next(&value);
        }
        Ok(result)
    }

    pub async fn like(&self, id: &str, on: bool) -> Result<(), String> {
        request(
            if on {
                reqwest::Method::POST
            } else {
                reqwest::Method::DELETE
            },
            &format!("/likes/tracks/{}", resource_path("tracks", id)?),
            &self.token().await?,
        )
        .await
        .map(|_| ())
    }

    pub async fn stream(&self, id: &str) -> Result<super::ResolvedStream, String> {
        let value = self
            .get(&format!("/tracks/{}/streams", resource_path("tracks", id)?))
            .await?;
        let stream = value["hls_aac_160_url"]
            .as_str()
            .ok_or("No full AAC stream is available for this track")?;
        let url = reqwest::Url::parse(stream).map_err(|_| "Invalid stream URL")?;
        if url.scheme() != "https" {
            return Err("Invalid stream URL".into());
        }
        let mut request = reqwest::Client::new()
            .get(url.clone())
            .timeout(std::time::Duration::from_secs(30));
        if url.host_str() == Some("api.soundcloud.com") {
            api_url(stream)?;
            request = request.header("Authorization", format!("OAuth {}", self.token().await?));
        } else if !url
            .host_str()
            .is_some_and(|host| host.ends_with(".sndcdn.com"))
        {
            return Err("Unexpected SoundCloud stream host".into());
        }
        let response = request
            .send()
            .await
            .map_err(|_| "SoundCloud stream request failed")?;
        if !response.status().is_success()
            || response.url().scheme() != "https"
            || !response
                .url()
                .host_str()
                .is_some_and(|host| host.ends_with(".sndcdn.com"))
        {
            return Err("SoundCloud did not return a playable CDN stream".into());
        }
        Ok(super::ResolvedStream::HlsAac(response.url().to_string()))
    }
}

fn resource_path(kind: &str, id: &str) -> Result<String, String> {
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Invalid SoundCloud resource ID".into());
    }
    Ok(urlencoding::encode(&format!("soundcloud:{kind}:{id}")).into_owned())
}

fn collection(value: &Value) -> &[Value] {
    value
        .get("collection")
        .unwrap_or(value)
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn tracks(value: &Value) -> Vec<reader::Track> {
    collection(value)
        .iter()
        .filter_map(super::parse_track_metadata)
        .collect()
}

fn next(value: &Value) -> Option<String> {
    value["next_href"]
        .as_str()
        .filter(|url| !url.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_api_tracks_have_urns_and_no_web_transcodings() {
        let tracks = tracks(
            &serde_json::json!({"collection":[{"urn":"soundcloud:tracks:123","title":"Song","duration":120000,"user":{"username":"Artist"}}]}),
        );
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].id.key(), "123");
        assert_eq!(tracks[0].duration, 120);
        assert_eq!(
            resource_path("tracks", "123").unwrap(),
            "soundcloud%3Atracks%3A123"
        );
    }

    #[test]
    fn cursors_cannot_forward_tokens_off_origin() {
        assert!(api_url("https://api.soundcloud.com/me?cursor=next").is_ok());
        for url in [
            "https://example.org/me",
            "//example.org/me",
            "http://api.soundcloud.com/me",
            "https://api.soundcloud.com:444/me",
            "https://user@api.soundcloud.com/me",
        ] {
            assert!(api_url(url).is_err());
        }
    }
}
