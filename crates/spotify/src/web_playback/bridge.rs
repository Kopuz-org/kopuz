//! Local HTTP bridge for Spotify Web Playback SDK.
//!
//! The Web Playback SDK requires a browser-like context. Rather than embed a
//! WebView (which would require pulling wry into the spotify crate), we run a
//! tiny HTTP server on `127.0.0.1` that serves the SDK host page and exposes
//! two endpoints the JS calls into:
//!
//!   GET  /            -> the SDK host HTML
//!   GET  /token       -> returns a fresh access token (JSON: {"access_token": "..."})
//!   POST /event       -> JS posts SDK events as JSON
//!
//! The host application opens `http://127.0.0.1:<port>/` in the user's default
//! browser. The browser (Chrome/Edge/Firefox/Safari) runs Spotify's SDK with
//! its real media/DRM stack. Tokens are never embedded in the HTML; the JS
//! calls /token on every Spotify SDK `getOAuthToken` request.

use crate::auth::AuthCore;
use crate::error::{Result, SpotifyError};
use crate::token_store::TokenStore;
use crate::web_playback::SPOTIFY_PLAYER_HTML;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, mpsc};

/// Events emitted by the JS page over the bridge.
#[derive(Debug, Clone)]
pub enum BridgeEvent {
    Ready { device_id: String },
    NotReady { device_id: String },
    PlayerStateChanged(serde_json::Value),
    AutoplayFailed,
    InitializationError(String),
    AuthenticationError(String),
    AccountError(String),
    PlaybackError(String),
    Activated { device_id: Option<String> },
}

impl BridgeEvent {
    pub fn from_named(name: &str, payload: serde_json::Value) -> Self {
        let s = |k: &str| {
            payload
                .get(k)
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        };
        match name {
            "ready" => BridgeEvent::Ready {
                device_id: s("device_id").unwrap_or_default(),
            },
            "not_ready" => BridgeEvent::NotReady {
                device_id: s("device_id").unwrap_or_default(),
            },
            "player_state_changed" => BridgeEvent::PlayerStateChanged(payload),
            "autoplay_failed" => BridgeEvent::AutoplayFailed,
            "initialization_error" => {
                BridgeEvent::InitializationError(s("message").unwrap_or_default())
            }
            "authentication_error" => {
                BridgeEvent::AuthenticationError(s("message").unwrap_or_default())
            }
            "account_error" => BridgeEvent::AccountError(s("message").unwrap_or_default()),
            "playback_error" => BridgeEvent::PlaybackError(s("message").unwrap_or_default()),
            "activated" => BridgeEvent::Activated {
                device_id: s("device_id"),
            },
            _ => BridgeEvent::PlaybackError(format!("unknown bridge event: {name}")),
        }
    }
}

/// Bridge owns the channel of inbound JS events and an `AuthCore` for token
/// supply. `start` runs a local HTTP server bound to `127.0.0.1` on the
/// requested port until the bridge is dropped or the server is shut down.
pub struct WebPlaybackBridge<S: TokenStore + 'static> {
    pub auth: Arc<AuthCore<S>>,
    pub device_name: String,
    tx: mpsc::UnboundedSender<BridgeEvent>,
    rx: Mutex<mpsc::UnboundedReceiver<BridgeEvent>>,
}

impl<S: TokenStore + 'static> WebPlaybackBridge<S> {
    pub fn new(auth: Arc<AuthCore<S>>, device_name: impl Into<String>) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            auth,
            device_name: device_name.into(),
            tx,
            rx: Mutex::new(rx),
        }
    }

    /// Test-only dispatch used by unit tests.
    #[cfg(test)]
    pub fn dispatch(&self, ev: BridgeEvent) {
        let _ = self.tx.send(ev);
    }

    /// Pull the next event. Returns `WebPlaybackUnavailable` if the channel
    /// has been closed.
    pub async fn next_event(&self) -> Result<BridgeEvent> {
        let mut rx = self.rx.lock().await;
        rx.recv().await.ok_or(SpotifyError::WebPlaybackUnavailable)
    }

    /// Start the local HTTP server. Returns the bound `SocketAddr` once the
    /// listener is open. The server runs until the returned task is aborted
    /// or the process exits.
    ///
    /// `port = 0` lets the OS choose a free port.
    pub async fn start(self: Arc<Self>, port: u16) -> Result<std::net::SocketAddr> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
        let addr = listener.local_addr()?;
        let me = self.clone();
        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((sock, _)) => {
                        let me2 = me.clone();
                        tokio::spawn(async move {
                            if let Err(e) = me2.handle_connection(sock).await {
                                tracing::debug!(target: "spotify::web_playback", "conn error: {e}");
                            }
                        });
                    }
                    Err(e) => {
                        tracing::warn!(target: "spotify::web_playback", "accept failed: {e}");
                        break;
                    }
                }
            }
        });
        Ok(addr)
    }

    async fn handle_connection(self: Arc<Self>, mut sock: TcpStream) -> Result<()> {
        let mut buf = vec![0u8; 16 * 1024];
        let n = sock.read(&mut buf).await?;
        let req = String::from_utf8_lossy(&buf[..n]).to_string();

        // Parse request line and headers minimally. We do not support keep-alive.
        let mut lines = req.split("\r\n");
        let first = lines.next().unwrap_or("");
        let mut parts = first.split_whitespace();
        let method = parts.next().unwrap_or("");
        let path = parts.next().unwrap_or("");

        // Find body after CRLF CRLF.
        let body = req.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");

        let response = match (method, path) {
            ("GET", "/") | ("GET", "/index.html") => html_response(SPOTIFY_PLAYER_HTML),
            ("GET", "/token") => match self.auth.refresh_if_needed().await {
                Ok(t) => json_response(200, &serde_json::json!({ "access_token": t.access_token })),
                Err(e) => json_response(500, &serde_json::json!({ "error": e.to_string() })),
            },
            ("GET", "/device-name") => {
                json_response(200, &serde_json::json!({ "name": self.device_name }))
            }
            ("POST", "/event") => {
                // Parse JSON body. The page sends { "name": "...", "payload": {...} }.
                match serde_json::from_str::<serde_json::Value>(body) {
                    Ok(v) => {
                        let name = v.get("name").and_then(|s| s.as_str()).unwrap_or("");
                        let payload = v.get("payload").cloned().unwrap_or(serde_json::json!({}));
                        let _ = self.tx.send(BridgeEvent::from_named(name, payload));
                        json_response(200, &serde_json::json!({ "ok": true }))
                    }
                    Err(e) => json_response(400, &serde_json::json!({ "error": e.to_string() })),
                }
            }
            _ => not_found(),
        };

        sock.write_all(response.as_bytes()).await?;
        let _ = sock.shutdown().await;
        Ok(())
    }
}

fn html_response(body: &str) -> String {
    let mut out = String::new();
    out.push_str("HTTP/1.1 200 OK\r\n");
    out.push_str("Content-Type: text/html; charset=utf-8\r\n");
    // Bridge endpoints are same-origin (the JS calls /token and /event on the
    // same 127.0.0.1:port the page came from), so no CORS preflight is needed.
    out.push_str(&format!("Content-Length: {}\r\n", body.len()));
    out.push_str("Cache-Control: no-store\r\n");
    out.push_str("Connection: close\r\n\r\n");
    out.push_str(body);
    out
}

fn json_response(status: u16, value: &serde_json::Value) -> String {
    let body = value.to_string();
    let status_text = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "OK",
    };
    let mut out = String::new();
    out.push_str(&format!("HTTP/1.1 {status} {status_text}\r\n"));
    out.push_str("Content-Type: application/json; charset=utf-8\r\n");
    out.push_str(&format!("Content-Length: {}\r\n", body.len()));
    out.push_str("Cache-Control: no-store\r\n");
    out.push_str("Connection: close\r\n\r\n");
    out.push_str(&body);
    out
}

fn not_found() -> String {
    let body = "not found";
    let mut out = String::new();
    out.push_str("HTTP/1.1 404 Not Found\r\n");
    out.push_str("Content-Type: text/plain\r\n");
    out.push_str(&format!("Content-Length: {}\r\n", body.len()));
    out.push_str("Connection: close\r\n\r\n");
    out.push_str(body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_named_events() {
        let ev = BridgeEvent::from_named("ready", serde_json::json!({"device_id": "ABC"}));
        match ev {
            BridgeEvent::Ready { device_id } => assert_eq!(device_id, "ABC"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn account_error_maps() {
        let ev =
            BridgeEvent::from_named("account_error", serde_json::json!({"message": "premium"}));
        assert!(matches!(ev, BridgeEvent::AccountError(_)));
    }

    #[test]
    fn html_response_has_correct_content_length() {
        let r = html_response("hello");
        assert!(r.contains("Content-Length: 5"));
        assert!(r.ends_with("hello"));
    }

    #[test]
    fn json_response_serializes_value() {
        let r = json_response(200, &serde_json::json!({"k": 1}));
        assert!(r.contains("Content-Type: application/json"));
        assert!(r.contains(r#"{"k":1}"#));
    }
}
