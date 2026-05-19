use crate::{RadioMetadata, RadioMetadataProvider};
use tokio::sync::mpsc;

pub struct ListenMoeProvider;

const STATION_NAME: &str = "Listen.Moe";

fn parse_track_update(d: &serde_json::Value) -> Option<RadioMetadata> {
    let song = d.get("song")?;

    let title = song
        .get("title")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("Unknown")
        .to_string();

    let artist = song
        .get("artists")
        .and_then(|v| v.as_array())
        .and_then(|artists| {
            let joined = artists
                .iter()
                .filter_map(|a| a.get("name").and_then(|n| n.as_str()))
                .filter(|s| !s.trim().is_empty())
                .collect::<Vec<_>>()
                .join(", ");
            if joined.is_empty() {
                None
            } else {
                Some(joined)
            }
        })
        .unwrap_or_else(|| "Unknown Artist".to_string());

    let cover_url = song
        .get("albums")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|album| album.get("image"))
        .and_then(|img| img.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| format!("https://cdn.listen.moe/covers/{}", s));

    Some(RadioMetadata {
        station: STATION_NAME.to_string(),
        title,
        artist,
        cover_url,
    })
}

#[cfg(not(target_arch = "wasm32"))]
impl RadioMetadataProvider for ListenMoeProvider {
    fn start(&self, stream_id: &str) -> mpsc::UnboundedReceiver<RadioMetadata> {
        use futures_util::{SinkExt, StreamExt};
        use serde::Deserialize;
        use tokio::time::{Duration, sleep};
        use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

        #[derive(Deserialize)]
        struct WsMessage {
            op: u8,
            t: Option<String>,
            d: Option<serde_json::Value>,
        }

        let (tx, rx) = mpsc::unbounded_channel();
        let ws_url = if stream_id == "listen_moe_kpop" || stream_id == "kpop" {
            "wss://listen.moe/kpop/gateway_v2"
        } else {
            "wss://listen.moe/gateway_v2"
        }
        .to_string();

        tokio::spawn(async move {
            loop {
                if tx.is_closed() {
                    break;
                }

                if let Ok((mut ws_stream, _)) = connect_async(&ws_url).await {
                    let mut heartbeat_interval = 15000;

                    if let Some(Ok(Message::Text(msg))) = ws_stream.next().await {
                        if let Ok(ws_msg) = serde_json::from_str::<WsMessage>(&msg) {
                            if ws_msg.op == 0 {
                                if let Some(d) = ws_msg.d {
                                    if let Some(hb) = d.get("heartbeat").and_then(|v| v.as_u64()) {
                                        heartbeat_interval = hb;
                                    } else {
                                        tracing::warn!(
                                            "[radio] ListenMoeProvider: missing heartbeat in welcome message, defaulting to {}ms",
                                            heartbeat_interval
                                        );
                                    }
                                }
                            }
                        }
                    }

                    let (mut write, mut read) = ws_stream.split();

                    let heartbeat_task = tokio::spawn(async move {
                        loop {
                            sleep(Duration::from_millis(heartbeat_interval)).await;
                            if write
                                .send(Message::Text("{\"op\":9}".to_string()))
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                    });

                    while let Some(Ok(msg)) = read.next().await {
                        if tx.is_closed() {
                            tracing::info!(
                                "[radio] ListenMoeProvider: tx is closed, exiting read loop"
                            );
                            break;
                        }

                        match msg {
                            Message::Text(text) => {
                                if let Ok(ws_msg) = serde_json::from_str::<WsMessage>(&text) {
                                    if ws_msg.op == 1 && ws_msg.t.as_deref() == Some("TRACK_UPDATE")
                                    {
                                        if let Some(d) = ws_msg.d {
                                            if let Some(meta) = parse_track_update(&d) {
                                                if tx.send(meta).is_err() {
                                                    tracing::warn!(
                                                        "[radio] ListenMoeProvider: tx send error, exiting"
                                                    );
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            Message::Close(_) => {
                                tracing::info!(
                                    "[radio] ListenMoeProvider: server closed connection"
                                );
                                break;
                            }
                            _ => {}
                        }
                    }

                    heartbeat_task.abort();
                    let _ = heartbeat_task.await;
                }

                sleep(Duration::from_secs(5)).await;
            }
        });

        rx
    }
}
