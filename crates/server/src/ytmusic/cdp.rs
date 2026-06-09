//! Minimal Chrome DevTools Protocol client — just enough to read cookies from a
//! live browser via `Storage.getCookies`. The YT Music sign-in flow uses this
//! instead of decrypting the profile's Cookies SQLite, which is unreadable on
//! Windows since Chrome 127's App-Bound Encryption (and saves us the per-OS
//! libsecret/Keychain/DPAPI decrypt dance). Works for any Chromium browser.

use std::path::Path;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

/// One cookie: (name, value, domain).
pub type Cookie = (String, String, String);

/// Browser DevTools websocket URL from the `DevToolsActivePort` file Chrome
/// writes once `--remote-debugging-port` is up (line 1 = port, line 2 = path).
/// `None` until that file appears.
async fn ws_url(profile: &Path) -> Option<String> {
    let txt = tokio::fs::read_to_string(profile.join("DevToolsActivePort"))
        .await
        .ok()?;
    let mut lines = txt.lines();
    let port = lines.next()?.trim();
    let path = lines.next()?.trim();
    Some(format!("ws://127.0.0.1:{port}{path}"))
}

/// Fetch all cookies from the live browser via CDP `Storage.getCookies`.
/// Returns an error (including "not ready yet") for the caller to poll on.
pub async fn get_cookies(profile: &Path) -> Result<Vec<Cookie>, String> {
    let url = ws_url(profile)
        .await
        .ok_or_else(|| "DevTools port not up yet".to_string())?;
    let (mut ws, _) = tokio_tungstenite::connect_async(&url)
        .await
        .map_err(|e| format!("CDP connect: {e}"))?;
    let req = json!({ "id": 1, "method": "Storage.getCookies" }).to_string();
    ws.send(Message::Text(req.into()))
        .await
        .map_err(|e| format!("CDP send: {e}"))?;
    while let Some(msg) = ws.next().await {
        let Message::Text(txt) = msg.map_err(|e| format!("CDP recv: {e}"))? else {
            continue;
        };
        let v: Value = serde_json::from_str(&txt).map_err(|e| format!("CDP parse: {e}"))?;
        if v.get("id").and_then(Value::as_u64) == Some(1) {
            let arr = v
                .pointer("/result/cookies")
                .and_then(Value::as_array)
                .ok_or("CDP: no cookies in result")?;
            return Ok(arr
                .iter()
                .filter_map(|c| {
                    Some((
                        c.get("name")?.as_str()?.to_string(),
                        c.get("value")?.as_str()?.to_string(),
                        c.get("domain")?.as_str()?.to_string(),
                    ))
                })
                .collect());
        }
    }
    Err("CDP: closed before reply".to_string())
}
