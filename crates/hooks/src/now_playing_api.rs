//! Now Playing API for widgets / OBS overlays (issue #466).
//!
//! A deliberately tiny HTTP server bound to 127.0.0.1 that answers
//! `GET /nowplaying` with a JSON snapshot of the current playback state. The
//! player task loop refreshes the snapshot; the server only ever reads it, so
//! it has no access to the player or the UI.

use std::sync::{OnceLock, RwLock};

use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct NowPlayingSnapshot {
    pub playing: bool,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// None for endless streams (radio).
    pub duration_secs: Option<u64>,
    pub position_secs: u64,
    /// Best cover reference we have: an http(s) URL or a local path/app URI.
    pub cover: Option<String>,
    pub source: Option<String>,
}

fn snapshot_cell() -> &'static RwLock<NowPlayingSnapshot> {
    static CELL: OnceLock<RwLock<NowPlayingSnapshot>> = OnceLock::new();
    CELL.get_or_init(|| RwLock::new(NowPlayingSnapshot::default()))
}

/// Called from the player task loop whenever state may have changed.
pub fn update(snapshot: NowPlayingSnapshot) {
    if let Ok(mut cell) = snapshot_cell().write() {
        *cell = snapshot;
    }
}

fn snapshot_json() -> String {
    let snapshot = snapshot_cell()
        .read()
        .map(|s| s.clone())
        .unwrap_or_default();
    serde_json::to_string(&snapshot).unwrap_or_else(|_| "{}".to_string())
}

/// Serve until the task is cancelled. Binding failures are logged, not fatal —
/// the feature is an optional extra.
pub async fn serve(port: u16) {
    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
        Ok(listener) => listener,
        Err(e) => {
            tracing::warn!(port, error = %e, "now-playing API failed to bind");
            return;
        }
    };
    tracing::info!(port, "now-playing API listening on 127.0.0.1");

    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            continue;
        };
        tokio::spawn(async move {
            // Read just enough for the request line; widgets send tiny GETs.
            let mut buffer = [0u8; 2048];
            let read = match stream.read(&mut buffer).await {
                Ok(n) if n > 0 => n,
                _ => return,
            };
            let request = String::from_utf8_lossy(&buffer[..read]);
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("/");
            let path = path.split('?').next().unwrap_or("/");

            let response = match path {
                "/" | "/nowplaying" | "/now-playing" => {
                    let body = snapshot_json();
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nCache-Control: no-store\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    )
                }
                _ => "HTTP/1.1 404 Not Found\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
                    .to_string(),
            };
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.shutdown().await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn serves_snapshot_json_and_404s_elsewhere() {
        update(NowPlayingSnapshot {
            playing: true,
            title: "Test Title".into(),
            artist: "Test Artist".into(),
            duration_secs: Some(180),
            position_secs: 42,
            ..Default::default()
        });
        let port = 39471;
        tokio::spawn(serve(port));

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let body = loop {
            if let Ok(mut stream) = tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
                stream
                    .write_all(b"GET /nowplaying HTTP/1.1\r\nHost: localhost\r\n\r\n")
                    .await
                    .unwrap();
                let mut buf = Vec::new();
                stream.read_to_end(&mut buf).await.unwrap();
                break String::from_utf8_lossy(&buf).to_string();
            }
            assert!(std::time::Instant::now() < deadline, "server never came up");
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        };
        assert!(body.starts_with("HTTP/1.1 200"), "{body}");
        assert!(body.contains("\"title\":\"Test Title\""), "{body}");
        assert!(body.contains("\"position_secs\":42"), "{body}");
        assert!(body.contains("Access-Control-Allow-Origin: *"), "{body}");

        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        stream
            .write_all(b"GET /other HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.unwrap();
        assert!(String::from_utf8_lossy(&buf).starts_with("HTTP/1.1 404"));
    }
}
