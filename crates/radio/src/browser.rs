//! Client for the community radio-browser.info station directory.
//!
//! Per API guidelines: mirrors are discovered at runtime, requests carry a
//! speaking user agent, and plays are reported via the click endpoint.

use crate::manifest::{MetadataSourceDef, StaticSourceDef, StationManifest, StreamDef};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::Duration;

/// Round-robin DNS entry point: mirror discovery, and fallback when
/// discovery fails.
const ENTRY_HOST: &str = "all.api.radio-browser.info";

/// The stream id used for every radio-browser station's single stream.
pub const BROWSER_STREAM_ID: &str = "main";

#[derive(Debug, thiserror::Error)]
pub enum BrowserError {
    #[error("Network error: {0}")]
    Network(String),
}

/// One station from the radio-browser JSON API. Every field defaulted; the
/// API has changed field types before, and empty values are common.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct BrowserStation {
    #[serde(default)]
    pub stationuuid: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub url: String,

    /// Stream URL with playlists (m3u/pls for example), already resolved.
    #[serde(default)]
    pub url_resolved: String,
    #[serde(default)]
    pub homepage: String,
    #[serde(default)]
    pub favicon: String,

    /// Comma-separated tag list.
    #[serde(default)]
    pub tags: String,
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub countrycode: String,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub codec: String,
    /// 1 when the station streams HLS.
    #[serde(default)]
    pub hls: u8,
    #[serde(default)]
    pub bitrate: u32,
    #[serde(default)]
    pub votes: u64,
    #[serde(default)]
    pub clickcount: u64,
}

#[derive(Debug, Deserialize)]
struct ServerEntry {
    #[serde(default)]
    name: String,
}

fn http_client() -> Result<reqwest::Client, BrowserError> {
    reqwest::Client::builder()
        .user_agent(format!("Kopuz/{}", env!("CARGO_PKG_VERSION")))
        // Short connect timeout so a dead address family
        // (mirrors publish AAAA records) falls back within the request budget.
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| BrowserError::Network(e.to_string()))
}

/// A mirror answering this cheap endpoint with a 2xx is considered healthy.
async fn is_healthy(client: &reqwest::Client, base: &str) -> bool {
    match client.get(format!("{base}/json/stats")).send().await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

/// First healthy mirror from the entry host's list, the entry host itself
/// last. `None` (whole pool down) is deliberately not cached so the next
/// request probes again.
async fn discover_base() -> Option<String> {
    let client = http_client().ok()?;

    let mut candidates: Vec<String> = match client
        .get(format!("https://{ENTRY_HOST}/json/servers"))
        .send()
        .await
    {
        Ok(resp) => resp
            .json::<Vec<ServerEntry>>()
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|s| s.name)
            .filter(|n| !n.is_empty())
            .collect(),
        Err(e) => {
            tracing::debug!("radio-browser server list fetch failed: {e}");
            Vec::new()
        }
    };

    // Spread load across mirrors without pulling in a rand dependency.
    let count = candidates.len();
    if count > 1 {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as usize)
            .unwrap_or(0);
        candidates.rotate_left(nanos % count);
    }
    candidates.push(ENTRY_HOST.to_string());
    candidates.dedup();

    for host in candidates {
        let base = format!("https://{host}");
        if is_healthy(&client, &base).await {
            tracing::debug!("radio-browser mirror selected: {host}");
            return Some(base);
        }
    }
    tracing::warn!("no reachable radio-browser server");
    None
}

static BASE: RwLock<Option<String>> = RwLock::new(None);

async fn base_url() -> Result<String, BrowserError> {
    if let Some(base) = BASE.read().ok().and_then(|g| g.clone()) {
        return Ok(base);
    }
    let base = discover_base().await.ok_or_else(|| {
        BrowserError::Network("no reachable radio-browser.info server".to_string())
    })?;
    if let Ok(mut guard) = BASE.write() {
        *guard = Some(base.clone());
    }
    Ok(base)
}

/// Forget the cached mirror so the next request rediscovers.
/// Called when a request against it fails.
fn invalidate_base() {
    if let Ok(mut guard) = BASE.write() {
        *guard = None;
    }
}

/// True when the station streams HLS, which the decoder can't play.
/// Isn't worth bothering with too.
pub fn is_hls(station: &BrowserStation) -> bool {
    station.hls != 0
        || station.codec.eq_ignore_ascii_case("hls")
        || [&station.url_resolved, &station.url]
            .iter()
            .any(|u| u.split(['?', '#']).next().unwrap_or("").ends_with(".m3u8"))
}

async fn fetch_stations_from(
    base: &str,
    path: &str,
    query: &[(&str, &str)],
) -> Result<Vec<BrowserStation>, BrowserError> {
    let client = http_client()?;
    let mut stations: Vec<BrowserStation> = client
        .get(format!("{base}{path}"))
        .query(query)
        .send()
        .await
        .and_then(|resp| resp.error_for_status())
        .map_err(|e| BrowserError::Network(e.to_string()))?
        .json()
        .await
        .map_err(|e| BrowserError::Network(e.to_string()))?;

    // HLS stations would only fail at play time; drop them up front.
    let before = stations.len();
    stations.retain(|s| !is_hls(s));
    if stations.len() < before {
        tracing::debug!(dropped = before - stations.len(), "filtered HLS stations");
    }
    Ok(stations)
}

async fn fetch_stations(
    path: &str,
    query: &[(&str, &str)],
) -> Result<Vec<BrowserStation>, BrowserError> {
    let base = base_url().await?;
    match fetch_stations_from(&base, path, query).await {
        Ok(stations) => Ok(stations),
        Err(first_err) => {
            // The cached mirror may have died; rediscover once and retry.
            invalidate_base();
            let retry_base = base_url().await.map_err(|_| first_err)?;
            fetch_stations_from(&retry_base, path, query).await
        }
    }
}

/// Most-listened stations, working streams only.
pub async fn top_stations(limit: u32) -> Result<Vec<BrowserStation>, BrowserError> {
    let limit = limit.to_string();
    fetch_stations(
        "/json/stations/search",
        &[
            ("order", "clickcount"),
            ("reverse", "true"),
            ("hidebroken", "true"),
            ("limit", &limit),
        ],
    )
    .await
}

/// Free-text station search ordered by popularity; working.
pub async fn search(query: &str, limit: u32) -> Result<Vec<BrowserStation>, BrowserError> {
    let limit = limit.to_string();
    fetch_stations(
        "/json/stations/search",
        &[
            ("name", query),
            ("order", "clickcount"),
            ("reverse", "true"),
            ("hidebroken", "true"),
            ("limit", &limit),
        ],
    )
    .await
}

/// Report a play for radio-browser's popularity data. Fire&Forget
pub fn count_click(stationuuid: &str) {
    let uuid = stationuuid.to_string();
    tokio::spawn(async move {
        let Ok(base) = base_url().await else { return };
        let Ok(client) = http_client() else { return };
        if let Err(e) = client.get(format!("{base}/json/url/{uuid}")).send().await {
            tracing::debug!("radio-browser click count failed: {e}");
        }
    });
}

/// Human-readable one-liner for station lists: "Country - CODEC 128 kbps".
pub fn station_detail(station: &BrowserStation) -> String {
    let mut parts = Vec::new();
    if !station.country.is_empty() {
        parts.push(station.country.clone());
    }
    match (station.codec.is_empty(), station.bitrate) {
        (false, 0) => parts.push(station.codec.clone()),
        (false, b) => parts.push(format!("{} {} kbps", station.codec, b)),
        (true, b) if b > 0 => parts.push(format!("{b} kbps")),
        _ => {}
    }
    parts.join(" - ")
}

/// Convert a radio-browser station into a playable manifest. Inserted into
/// the live registry at play time, skipping registry JSON validation, so
/// plain-http streams (suprisingly common here) stay playable.
pub fn to_manifest(station: &BrowserStation) -> StationManifest {
    let stream_url = if station.url_resolved.is_empty() {
        station.url.clone()
    } else {
        station.url_resolved.clone()
    };

    // Favicons double as cover art;
    // only https survives the static-metadata contract, and mixed-content rules on webviews.
    let cover_url = Some(station.favicon.clone()).filter(|f| f.starts_with("https://"));

    let artist = if station.country.is_empty() {
        "Live Radio".to_string()
    } else {
        station.country.clone()
    };

    StationManifest {
        schema_version: "1.0".to_string(),
        id: station.stationuuid.clone(),
        name: station.name.trim().to_string(),
        description: station_detail(station),
        icon: "fa-solid fa-radio".to_string(),
        tags: station
            .tags
            .split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect(),
        streams: vec![StreamDef {
            id: BROWSER_STREAM_ID.to_string(),
            name: "Live".to_string(),
            url: stream_url,
            codec: Some(station.codec.clone()).filter(|c| !c.is_empty()),
            bitrate: Some(station.bitrate).filter(|b| *b > 0),
            icon: None,
        }],
        metadata: Some(MetadataSourceDef::Static(StaticSourceDef {
            title: station.name.trim().to_string(),
            artist,
            cover_url,
            stream_overrides: HashMap::new(),
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_station() -> BrowserStation {
        serde_json::from_str(
            r#"{
                "stationuuid": "00000000-0000-4000-8000-000000000001",
                "name": " Test FM ",
                "url": "https://radio.example.org/test.pls",
                "url_resolved": "https://stream.example.org/test-128-mp3",
                "homepage": "https://radio.example.org/",
                "favicon": "https://radio.example.org/icon.png",
                "tags": "ambient, chillout,downtempo,",
                "country": "Testland",
                "countrycode": "TL",
                "language": "english",
                "codec": "MP3",
                "bitrate": 128,
                "votes": 1337,
                "clickcount": 4242
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn deserializes_station_with_missing_fields() {
        let st: BrowserStation =
            serde_json::from_str(r#"{"stationuuid": "abc", "name": "X"}"#).unwrap();
        assert_eq!(st.stationuuid, "abc");
        assert_eq!(st.bitrate, 0);
        assert!(st.favicon.is_empty());
    }

    #[test]
    fn converts_station_to_manifest() {
        let manifest = to_manifest(&sample_station());
        assert_eq!(manifest.id, "00000000-0000-4000-8000-000000000001");
        assert_eq!(manifest.name, "Test FM");
        assert_eq!(manifest.streams.len(), 1);
        let stream = &manifest.streams[0];
        assert_eq!(stream.id, BROWSER_STREAM_ID);
        assert_eq!(stream.url, "https://stream.example.org/test-128-mp3");
        assert_eq!(stream.bitrate, Some(128));
        assert_eq!(manifest.tags, vec!["ambient", "chillout", "downtempo"]);

        let Some(MetadataSourceDef::Static(meta)) = &manifest.metadata else {
            panic!("expected static metadata");
        };
        assert_eq!(meta.title, "Test FM");
        assert_eq!(meta.artist, "Testland");
        assert_eq!(
            meta.cover_url.as_deref(),
            Some("https://radio.example.org/icon.png")
        );
    }

    #[test]
    fn manifest_falls_back_to_raw_url_and_drops_http_favicon() {
        let mut st = sample_station();
        st.url_resolved.clear();
        st.favicon = "http://insecure.example/icon.png".to_string();
        st.country.clear();
        let manifest = to_manifest(&st);
        assert_eq!(
            manifest.streams[0].url,
            "https://radio.example.org/test.pls"
        );
        let Some(MetadataSourceDef::Static(meta)) = &manifest.metadata else {
            panic!("expected static metadata");
        };
        assert_eq!(meta.cover_url, None);
        assert_eq!(meta.artist, "Live Radio");
    }

    #[test]
    fn detects_hls_stations() {
        assert!(!is_hls(&sample_station()));

        let mut st = sample_station();
        st.hls = 1;
        assert!(is_hls(&st));

        let mut st = sample_station();
        st.codec = "HLS".to_string();
        assert!(is_hls(&st));

        let mut st = sample_station();
        st.url_resolved = "https://stream.example.org/live/master.m3u8?token=x".to_string();
        assert!(is_hls(&st));

        let mut st = sample_station();
        st.url_resolved = "https://stream.example.org/live.m3u".to_string();
        assert!(!is_hls(&st));
    }

    #[test]
    fn station_detail_formats() {
        assert_eq!(station_detail(&sample_station()), "Testland - MP3 128 kbps");
        let mut st = sample_station();
        st.bitrate = 0;
        assert_eq!(station_detail(&st), "Testland - MP3");
        st.codec.clear();
        st.country.clear();
        assert_eq!(station_detail(&st), "");
    }
}
