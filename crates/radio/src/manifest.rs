use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

fn default_icon() -> String {
    "fa-solid fa-radio".to_string()
}

fn default_poll_secs() -> u64 {
    5
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StationManifest {
    pub schema_version: String,
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default = "default_icon")]
    pub icon: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub streams: Vec<StreamDef>,
    /// How to fetch now-playing metadata for this station
    pub metadata: Option<MetadataSourceDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamDef {
    pub id: String,
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub codec: Option<String>,
    #[serde(default)]
    pub bitrate: Option<u32>,
    #[serde(default)]
    pub icon: Option<String>,
}

/// WebSocket OR REST
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum MetadataSourceDef {
    #[serde(rename = "websocket")]
    WebSocket(WebSocketSourceDef),
    #[serde(rename = "rest")]
    Rest(RestSourceDef),
}

// Keep in minds that this wrote entirely for listen.moe, haven't tested with other providers that use websocket.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSocketSourceDef {
    /// URL template, e.g. "wss://listen.moe/{stream_key}/gateway_v2"
    pub url: String,
    #[serde(default)]
    pub stream_url_map: HashMap<String, String>,
    #[serde(default)]
    pub message_filter: Option<WsMessageFilter>,
    #[serde(default)]
    pub heartbeat: Option<WsHeartbeat>,
    pub mapping: FieldMapping,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsMessageFilter {
    pub op_field: String,
    pub op_value: u64,
    pub type_field: Option<String>,
    pub type_value: Option<String>,
    pub data_field: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsHeartbeat {
    pub message: String,
    pub interval_field: String,
    pub default_interval_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestSourceDef {
    /// default metadata URL; used when stream_url_map has no override.
    pub url: String,
    /// per-stream metadata URL overrides (stream_id → full URL).
    /// when present, the matching URL is used instead of `url`.
    #[serde(default)]
    pub stream_url_map: HashMap<String, String>,
    #[serde(default = "default_poll_secs")]
    pub poll_interval_secs: u64,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// when the API returns an array with entries for multiple channels,
    /// use this to select the correct entry for the active stream.
    #[serde(default)]
    pub entry_selector: Option<EntrySelector>,
    /// Maps stream IDs to display names for entry_selector matching
    /// when match_value_from is StreamName.
    #[serde(default)]
    pub stream_name_map: HashMap<String, String>,
    pub mapping: FieldMapping,
}

/// User-defined paths to extract metadata from JSON responses
/// Uses dot-notation: "song.title", "song.artists.0.name"
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldMapping {
    pub title: String,
    pub artist: String,
    #[serde(default)]
    pub artwork_url: Option<String>,
    #[serde(default)]
    pub artwork_url_template: Option<String>,
    #[serde(default)]
    pub artist_separator: Option<String>,
    #[serde(default)]
    pub artist_array_field: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("Unsupported schema version: {0}")]
    UnsupportedSchemaVersion(String),
    #[error("Manifest ID must contain only alphanumeric characters, underscores, and dashes")]
    InvalidId,
    #[error("Manifest name cannot be empty")]
    EmptyName,
    #[error("Manifest description cannot be empty")]
    EmptyDescription,
    #[error("Stream URLs must use https:// or wss:// scheme. Invalid URL: {0}")]
    InsecureUrl(String),
    #[error("Manifest must contain at least one stream")]
    NoStreams,
    #[error("Stream ID cannot be empty")]
    EmptyStreamId,
    #[error("Duplicate stream ID: {0}")]
    DuplicateStreamId(String),
}

impl StationManifest {
    pub fn validate(&self) -> Result<(), ManifestError> {
        if !matches!(self.schema_version.as_str(), "1" | "1.0") {
            return Err(ManifestError::UnsupportedSchemaVersion(self.schema_version.clone()));
        }
        // ID check
        if self.id.trim().is_empty()
            || !self
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(ManifestError::InvalidId);
        }

        if self.name.trim().is_empty() {
            return Err(ManifestError::EmptyName);
        }

        if self.description.trim().is_empty() {
            return Err(ManifestError::EmptyDescription);
        }

        if self.streams.is_empty() {
            return Err(ManifestError::NoStreams);
        }

        let mut seen_stream_ids: HashSet<String> = HashSet::new();

        for stream in &self.streams {
            if stream.id.trim().is_empty() {
                return Err(ManifestError::EmptyStreamId);
            }
            if !seen_stream_ids.insert(stream.id.clone()) {
                return Err(ManifestError::DuplicateStreamId(stream.id.clone()));
            }
            if !stream.url.starts_with("https://") && !stream.url.starts_with("wss://") {
                return Err(ManifestError::InsecureUrl(stream.url.clone()));
            }
        }

        if let Some(meta) = &self.metadata {
            match meta {
                MetadataSourceDef::WebSocket(ws) => {
                    if !ws.url.starts_with("wss://") {
                        return Err(ManifestError::InsecureUrl(ws.url.clone()));
                    }
                    for url in ws.stream_url_map.values() {
                        if !url.starts_with("wss://") {
                            return Err(ManifestError::InsecureUrl(url.clone()));
                        }
                    }
                }
                MetadataSourceDef::Rest(rest) => {
                    if !rest.url.starts_with("https://") {
                        return Err(ManifestError::InsecureUrl(rest.url.clone()));
                    }
                    for url in rest.stream_url_map.values() {
                        if !url.starts_with("https://") {
                            return Err(ManifestError::InsecureUrl(url.clone()));
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MatchValueFrom {
    /// match against the stream's display name from stream_name_map (e.g. "J1 HITS")
    #[default]
    StreamName,
    /// match against the raw stream id (e.g. "J1HITS")
    StreamId,
}

/// selects one entry from an array by matching a field value.
/// used when a single API response contains data for multiple streams/channels.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntrySelector {
    /// Dot-path to the array within the response (e.g. "station")
    pub array_path: String,
    /// Field within each array element to match against (e.g. "name")
    pub match_field: String,
    #[serde(default)]
    pub match_value_from: MatchValueFrom,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_manifest() {
        let json = r#"{
            "schema_version": "1.0",
            "id": "test_station",
            "name": "Test",
            "description": "Test description",
            "streams": [
                {
                    "id": "main",
                    "name": "Main",
                    "url": "https://example.com/stream"
                }
            ],
            "metadata": {
                "type": "rest",
                "url": "https://api.example.com",
                "mapping": {
                    "title": "title",
                    "artist": "artist"
                }
            }
        }"#;

        let manifest: StationManifest = serde_json::from_str(json).unwrap();
        assert!(manifest.validate().is_ok());
    }

    #[test]
    fn test_invalid_fields() {
        let mut manifest = StationManifest {
            schema_version: "1.0".into(),
            id: "test station !".into(),
            name: "Test".into(),
            description: "Test".into(),
            icon: default_icon(),
            tags: vec![],
            streams: vec![StreamDef {
                id: "main".into(),
                name: "Main".into(),
                url: "https://example.com".into(),
                codec: None,
                bitrate: None,
                icon: None,
            }],
            metadata: None,
        };

        assert!(matches!(manifest.validate(), Err(ManifestError::InvalidId)));

        manifest.id = "valid_id".into();
        manifest.name = "   ".into();
        assert!(matches!(manifest.validate(), Err(ManifestError::EmptyName)));

        manifest.name = "Valid Name".into();
        manifest.description = "   ".into();
        assert!(matches!(
            manifest.validate(),
            Err(ManifestError::EmptyDescription)
        ));
    }

    #[test]
    fn test_insecure_url() {
        let json = r#"{
            "schema_version": "1.0",
            "id": "test",
            "name": "Test",
            "description": "Test",
            "streams": [
                {
                    "id": "main",
                    "name": "Main",
                    "url": "http://example.com/stream"
                }
            ]
        }"#;

        let manifest: StationManifest = serde_json::from_str(json).unwrap();
        assert!(matches!(
            manifest.validate(),
            Err(ManifestError::InsecureUrl(_))
        ));
    }

    #[test]
    fn test_rest_with_entry_selector() {
        let json = r#"{
            "schema_version": "1.0",
            "id": "j1",
            "name": "J1 Tokyo",
            "description": "J1 FM",
            "streams": [
                { "id": "J1HITS", "name": "J1 HITS", "url": "https://example.com/hits" },
                { "id": "J1GOLD", "name": "J1 GOLD", "url": "https://example.com/gold" }
            ],
            "metadata": {
                "type": "rest",
                "url": "https://api.example.com/nowplaying",
                "entry_selector": {
                    "array_path": "station",
                    "match_field": "name",
                    "match_value_from": "stream_name"
                },
                "stream_name_map": {
                    "J1HITS": "J1 HITS",
                    "J1GOLD": "J1 GOLD"
                },
                "mapping": {
                    "title": "title",
                    "artist": "artist",
                    "artwork_url": "image_url"
                }
            }
        }"#;

        let manifest: StationManifest = serde_json::from_str(json).unwrap();
        assert!(manifest.validate().is_ok());

        let meta = manifest.metadata.unwrap();
        match meta {
            MetadataSourceDef::Rest(rest) => {
                let sel = rest.entry_selector.unwrap();
                assert_eq!(sel.array_path, "station");
                assert_eq!(sel.match_field, "name");
                assert_eq!(sel.match_value_from, MatchValueFrom::StreamName);
                assert_eq!(rest.stream_name_map.get("J1HITS").unwrap(), "J1 HITS");
                assert_eq!(rest.stream_name_map.get("J1GOLD").unwrap(), "J1 GOLD");
            }
            _ => panic!("expected Rest"),
        }
    }

    #[test]
    fn test_rest_with_stream_url_map() {
        let json = r#"{
            "schema_version": "1.0",
            "id": "adr",
            "name": "Asia DREAM Radio",
            "description": "ADR",
            "streams": [
                { "id": "japan_hits", "name": "Japan Hits", "url": "https://example.com/hits" },
                { "id": "jazz_sakura", "name": "Jazz Sakura", "url": "https://example.com/jazz" }
            ],
            "metadata": {
                "type": "rest",
                "url": "https://api.example.com/default",
                "stream_url_map": {
                    "japan_hits": "https://api.example.com/server1",
                    "jazz_sakura": "https://api.example.com/server2"
                },
                "poll_interval_secs": 10,
                "mapping": {
                    "title": "results.0.title",
                    "artist": "results.0.author",
                    "artwork_url": "results.0.img_large_url"
                }
            }
        }"#;

        let manifest: StationManifest = serde_json::from_str(json).unwrap();
        assert!(manifest.validate().is_ok());

        match manifest.metadata.unwrap() {
            MetadataSourceDef::Rest(rest) => {
                assert_eq!(rest.stream_url_map.len(), 2);
                assert_eq!(
                    rest.stream_url_map.get("japan_hits").unwrap(),
                    "https://api.example.com/server1"
                );
                assert_eq!(rest.poll_interval_secs, 10);
            }
            _ => panic!("expected Rest"),
        }
    }

    /// stream_url_map and entry_selector default to empty/None when omitted
    #[test]
    fn test_rest_defaults() {
        let json = r#"{
            "schema_version": "1.0",
            "id": "simple",
            "name": "Simple",
            "description": "A single-stream station",
            "streams": [
                { "id": "main", "name": "Main", "url": "https://example.com/stream" }
            ],
            "metadata": {
                "type": "rest",
                "url": "https://api.example.com",
                "mapping": { "title": "title", "artist": "artist" }
            }
        }"#;

        let manifest: StationManifest = serde_json::from_str(json).unwrap();
        match manifest.metadata.unwrap() {
            MetadataSourceDef::Rest(rest) => {
                assert!(rest.stream_url_map.is_empty());
                assert!(rest.entry_selector.is_none());
                assert!(rest.stream_name_map.is_empty());
                assert_eq!(rest.poll_interval_secs, 5); // default
            }
            _ => panic!("expected Rest"),
        }
    }
}
