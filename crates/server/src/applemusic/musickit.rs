//! MusicKit JS authorization in the selected external browser.

use crate::browser_auth::{self, Loopback};

const PREFIX: &str = "kopuz:musickit:v1:";
pub const ORIGIN: &str = "http://127.0.0.1:8900";

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Session {
    pub developer_token: String,
    pub music_user_token: String,
}

impl Session {
    pub fn unpack(value: &str) -> Option<Self> {
        serde_json::from_str(value.strip_prefix(PREFIX)?).ok()
    }

    pub fn pack(&self) -> Result<String, String> {
        serde_json::to_string(self)
            .map(|json| format!("{PREFIX}{json}"))
            .map_err(|e| e.to_string())
    }
}

pub async fn sign_in(
    developer_token: &str,
    open_browser: impl FnOnce(&str) -> Result<(), String>,
) -> Result<String, String> {
    if developer_token.trim().is_empty() {
        return Err("Enter your Apple Music developer token first".into());
    }
    let listener = Loopback::bind(8900).await?;
    let state = browser_auth::nonce();
    let path = format!("/sign-in/{state}");
    let config = serde_json::json!({"developerToken":developer_token,"state":state});
    let config = serde_json::to_string(&config)
        .map_err(|e| e.to_string())?
        .replace('<', "\\u003c");
    let page = include_str!("musickit.html").replace("__KOPUZ_CONFIGURATION__", &config);
    open_browser(&format!("{}{path}", listener.origin))?;
    let token = tokio::time::timeout(std::time::Duration::from_secs(300), async {
        loop {
            let request = listener.request().await?;
            if request.method == "GET" && request.target == path {
                request
                    .respond("200 OK", "text/html; charset=utf-8", &page)
                    .await;
                continue;
            }
            if request.method != "POST"
                || request.target != "/callback"
                || request.origin.as_deref() != Some(ORIGIN)
            {
                request
                    .respond("404 Not Found", "text/plain", "Not found")
                    .await;
                continue;
            }
            #[derive(serde::Deserialize)]
            struct Callback {
                state: String,
                token: Option<String>,
                error: Option<String>,
            }
            let callback: Result<Callback, _> = serde_json::from_slice(&request.body);
            let Ok(callback) = callback else {
                request
                    .respond("400 Bad Request", "text/plain", "Invalid callback")
                    .await;
                continue;
            };
            if callback.state != state {
                request
                    .respond("400 Bad Request", "text/plain", "Invalid state")
                    .await;
                continue;
            }
            if callback.error.is_some() {
                request.respond("200 OK", "text/plain", "Cancelled").await;
                return Err("Apple Music authorization was cancelled".to_string());
            }
            let Some(token) = callback.token.filter(|token| !token.is_empty()) else {
                request
                    .respond("400 Bad Request", "text/plain", "Missing token")
                    .await;
                continue;
            };
            request
                .respond("200 OK", "text/plain", "Authorization received")
                .await;
            return Ok(token);
        }
    })
    .await
    .map_err(|_| "Apple Music sign-in timed out. Return to Kopuz and retry.")??;
    let response = reqwest::Client::new()
        .get("https://api.music.apple.com/v1/me/storefront")
        .bearer_auth(developer_token)
        .header("Music-User-Token", &token)
        .header("Origin", ORIGIN)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|_| "Apple Music could not validate authorization")?;
    if !response.status().is_success() {
        return Err(format!(
            "Apple Music authorization failed ({})",
            response.status()
        ));
    }
    Session {
        developer_token: developer_token.to_string(),
        music_user_token: token,
    }
    .pack()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registered_sessions_do_not_masquerade_as_web_cookies() {
        assert!(Session::unpack("web-cookie").is_none());
        let session = Session {
            developer_token: "developer".into(),
            music_user_token: "subscriber".into(),
        };
        let restored = Session::unpack(&session.pack().unwrap()).unwrap();
        assert_eq!(restored.developer_token, "developer");
        assert_eq!(restored.music_user_token, "subscriber");
    }
}
