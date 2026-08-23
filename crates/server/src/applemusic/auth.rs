use std::sync::OnceLock;

use regex::Regex;

static BEARER_TOKEN: OnceLock<String> = OnceLock::new();

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

pub async fn get_bearer_token() -> Result<String, String> {
    if let Some(t) = BEARER_TOKEN.get() {
        tracing::debug!("am.auth: returning cached bearer token (len={})", t.len());
        return Ok(t.clone());
    }
    tracing::debug!("am.auth: scraping bearer token from music.apple.com");

    let client = reqwest::Client::new();
    let resp = client
        .get("https://music.apple.com")
        .header("User-Agent", USER_AGENT)
        .send()
        .await
        .map_err(|e| {
            tracing::warn!("am.auth: fetch music.apple.com failed: {e}");
            format!("fetch music.apple.com: {e}")
        })?;

    tracing::debug!("am.auth: music.apple.com → {}", resp.status());

    let body = resp.text().await.map_err(|e| {
        tracing::warn!("am.auth: read body failed: {e}");
        format!("read music.apple.com body: {e}")
    })?;

    tracing::debug!("am.auth: body length={}", body.len());

    let js_re = Regex::new(r"/assets/index~[^/]+\.js").unwrap();
    let js_path = match js_re.find(&body) {
        Some(m) => m.as_str(),
        None => {
            tracing::warn!(
                "am.auth: no index~*.js found in HTML (body_len={})",
                body.len()
            );
            return Err("no index~*.js found on music.apple.com".to_string());
        }
    };

    tracing::debug!("am.auth: found JS bundle path: {js_path}");

    let js_resp = client
        .get(format!("https://music.apple.com{js_path}"))
        .header("User-Agent", USER_AGENT)
        .send()
        .await
        .map_err(|e| {
            tracing::warn!("am.auth: fetch JS bundle failed: {e}");
            format!("fetch JS bundle: {e}")
        })?;

    tracing::debug!("am.auth: JS bundle → {}", js_resp.status());

    let js_body = js_resp.text().await.map_err(|e| {
        tracing::warn!("am.auth: read JS body failed: {e}");
        format!("read JS bundle: {e}")
    })?;

    tracing::debug!("am.auth: JS bundle length={}", js_body.len());

    let jwt_re = Regex::new(r"eyJ[A-Za-z0-9\-_]+\.[A-Za-z0-9\-_]+\.[A-Za-z0-9\-_]+").unwrap();
    let token = match jwt_re.find(&js_body) {
        Some(m) => m.as_str().to_string(),
        None => {
            tracing::warn!(
                "am.auth: no Bearer JWT found in JS bundle (len={})",
                js_body.len()
            );
            return Err("no Bearer JWT found in JS bundle".to_string());
        }
    };

    tracing::debug!("am.auth: scraped bearer token (len={})", token.len());
    let _ = BEARER_TOKEN.set(token.clone());
    Ok(token)
}
