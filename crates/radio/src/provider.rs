use crate::manifest::{MetadataSourceDef, RestSourceDef, StationManifest, WebSocketSourceDef};
use crate::mapping::{extract_artist, extract_artwork, extract_title, extract_value, select_entry};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RadioMetadata {
    pub station: String,
    pub title: String,
    pub artist: String,
    pub cover_url: Option<String>,
}

pub trait RadioMetadataProvider: Send + Sync {
    fn start(&self, stream_id: &str) -> mpsc::UnboundedReceiver<RadioMetadata>;
}

pub struct DynamicProvider {
    manifest: StationManifest,
}

impl DynamicProvider {
    pub fn new(manifest: StationManifest) -> Self {
        Self { manifest }
    }

    pub fn stream_url(&self, stream_id: &str) -> Option<String> {
        self.manifest
            .streams
            .iter()
            .find(|s| s.id == stream_id)
            .map(|s| s.url.clone())
    }
}

impl RadioMetadataProvider for DynamicProvider {
    fn start(&self, stream_id: &str) -> mpsc::UnboundedReceiver<RadioMetadata> {
        let (tx, rx) = mpsc::unbounded_channel();
        let stream_id = stream_id.to_string();

        if let Some(meta) = &self.manifest.metadata {
            let station_name = self.manifest.name.clone();
            match meta {
                MetadataSourceDef::Rest(rest_def) => {
                    let rest_def = rest_def.clone();
                    tokio::spawn(async move {
                        start_rest_metadata(rest_def, stream_id, station_name, tx).await;
                    });
                }
                MetadataSourceDef::WebSocket(ws_def) => {
                    let ws_def = ws_def.clone();
                    tokio::spawn(async move {
                        start_ws_metadata(ws_def, stream_id, station_name, tx).await;
                    });
                }
            }
        }

        rx
    }
}

async fn start_rest_metadata(
    def: RestSourceDef,
    stream_id: String,
    station_name: String,
    tx: mpsc::UnboundedSender<RadioMetadata>,
) {
    let client = reqwest::Client::builder()
        .user_agent(format!("Kopuz/{}", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    let url = def.stream_url_map.get(&stream_id)
        .cloned()
        .unwrap_or_else(|| def.url.replace("{stream_id}", &stream_id));
    let mut interval = time::interval(Duration::from_secs(def.poll_interval_secs));
    let mut last_title = String::new();

    loop {
        interval.tick().await;

        if tx.is_closed() {
            break;
        }

        let mut req = client.get(&url);
        for (k, v) in &def.headers {
            req = req.header(k, v);
        }

        match req.send().await {
            Ok(resp) => {
                if let Ok(json) = resp.json::<Value>().await {
                    let data: &Value = def
                        .entry_selector
                        .as_ref()
                        .and_then(|sel| select_entry(&json, sel, &stream_id, &def.stream_name_map))
                        .unwrap_or(&json);

                    let title = extract_title(data, &def.mapping);

                    if title != last_title && !title.is_empty() && title != "Unknown" {
                        last_title = title.clone();

                        let artist = extract_artist(data, &def.mapping);
                        let cover_url = extract_artwork(data, &def.mapping);

                        if tx.send(RadioMetadata { station: station_name.clone(), title, artist, cover_url }).is_err() {
                            break;
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!("REST provider fetch error: {}", e);
            }
        }
    }
}

fn process_ws_message(json: &Value, def: &WebSocketSourceDef, station_name: &str) -> Option<RadioMetadata> {
    let mut matches = true;
    if let Some(filter) = &def.message_filter {
        if let Some(op) = extract_value(json, &filter.op_field).and_then(|v| v.as_u64()) {
            if op != filter.op_value {
                matches = false;
            }
        } else {
            matches = false;
        }

        if let (Some(tf), Some(tv)) = (&filter.type_field, &filter.type_value) {
            if let Some(t) = extract_value(json, tf).and_then(|v| v.as_str()) {
                if t != tv {
                    matches = false;
                }
            } else {
                matches = false;
            }
        }
    }

    if matches {
        let data_root = if let Some(filter) = &def.message_filter {
            if let Some(df) = &filter.data_field {
                extract_value(json, df).unwrap_or(json)
            } else {
                json
            }
        } else {
            json
        };

        let title = extract_title(data_root, &def.mapping);
        let artist = extract_artist(data_root, &def.mapping);
        let cover_url = extract_artwork(data_root, &def.mapping);

        Some(RadioMetadata { station: station_name.to_string(), title, artist, cover_url })
    } else {
        None
    }
}

fn check_heartbeat(json: &Value, def: &WebSocketSourceDef, current_interval: u64) -> Option<u64> {
    if let Some(hb) = &def.heartbeat {
        if let Some(val) = extract_value(json, &hb.interval_field) {
            if let Some(ms) = val.as_u64() {
                if ms > 0 && ms != current_interval {
                    return Some(ms);
                }
            }
        }
    }
    None
}

// Keep in minds that this function wrote entirely for listen.moe, haven't tested with other providers that use websocket.
async fn start_ws_metadata(
    def: WebSocketSourceDef,
    stream_id: String,
    station_name: String,
    tx: mpsc::UnboundedSender<RadioMetadata>,
) {
    let url = def.stream_url_map.get(&stream_id).unwrap_or(&def.url).clone();

    loop {
        if tx.is_closed() {
            return;
        }
        tracing::debug!("Connecting to WebSocket: {}", url);
        match tokio_tungstenite::connect_async(&url).await {
            Ok((mut ws_stream, _)) => {
                tracing::debug!("Connected to {}", url);

                let mut heartbeat_interval_ms = def
                    .heartbeat
                    .as_ref()
                    .map(|h| h.default_interval_ms)
                    .unwrap_or(15000)
                    .max(1);

                let mut heartbeat_timer = time::interval(Duration::from_millis(heartbeat_interval_ms));
                heartbeat_timer.tick().await;

                loop {
                    if tx.is_closed() {
                        return;
                    }
                    tokio::select! {
                        _ = heartbeat_timer.tick(), if def.heartbeat.is_some() => {
                            if let Some(hb) = &def.heartbeat {
                                if let Err(e) = ws_stream.send(tokio_tungstenite::tungstenite::Message::Text(hb.message.clone())).await {
                                    tracing::warn!("WebSocket heartbeat failed: {}", e);
                                    break;
                                }
                            }
                        }
                        msg = ws_stream.next() => {
                            match msg {
                                Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) => {
                                    if let Ok(json) = serde_json::from_str::<Value>(&text) {
                                        if let Some(new_interval) = check_heartbeat(&json, &def, heartbeat_interval_ms) {
                                            heartbeat_interval_ms = new_interval;
                                            heartbeat_timer = time::interval(Duration::from_millis(heartbeat_interval_ms));
                                            heartbeat_timer.tick().await;
                                        }

                                        if let Some(meta) = process_ws_message(&json, &def, &station_name) {
                                            if tx.send(meta).is_err() {
                                                return;
                                            }
                                        }
                                    }
                                }
                                Some(Ok(_)) => {}
                                Some(Err(e)) => {
                                    tracing::warn!("WebSocket error: {}", e);
                                    break;
                                }
                                None => {
                                    tracing::warn!("WebSocket closed");
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!("WebSocket connection failed: {}", e);
            }
        }
        if tx.is_closed() {
            return;
        }
        // Wait before reconnecting
        time::sleep(Duration::from_secs(5)).await;
    }
}
