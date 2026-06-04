//! Typed Spotify Web API client.
//!
//! All requests are sent with a Bearer access token. On HTTP 401 the client
//! refreshes the token through the supplied `AuthCore` and retries once.

use crate::auth::AuthCore;
use crate::error::{Result, SpotifyError, classify_http};
use crate::token_store::TokenStore;
use crate::types::TrackMetadata;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const API_BASE: &str = "https://api.spotify.com/v1";

/// Minimal, defensive deserializations: every field is `Option`-friendly so a
/// missing or oddly-typed field cannot panic at runtime.
#[derive(Debug, Clone, Deserialize)]
pub struct UserProfile {
    pub id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub product: Option<String>,
    #[serde(default)]
    pub country: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Image {
    pub url: String,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ArtistRef {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub uri: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AlbumRef {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub uri: Option<String>,
    #[serde(default)]
    pub images: Vec<Image>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExternalUrls {
    #[serde(default)]
    pub spotify: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Restrictions {
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LinkedFromRef {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub uri: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Track {
    pub id: String,
    pub uri: String,
    pub name: String,
    #[serde(default)]
    pub duration_ms: u64,
    #[serde(default)]
    pub explicit: bool,
    #[serde(default)]
    pub artists: Vec<ArtistRef>,
    #[serde(default)]
    pub album: Option<AlbumRef>,
    #[serde(default)]
    pub external_urls: Option<ExternalUrls>,
    #[serde(default)]
    pub is_playable: Option<bool>,
    #[serde(default)]
    pub restrictions: Option<Restrictions>,
    #[serde(default)]
    pub linked_from: Option<LinkedFromRef>,
}

impl Track {
    pub fn into_metadata(self) -> TrackMetadata {
        TrackMetadata {
            id: self.id,
            uri: self.uri,
            title: self.name,
            artist_names: self.artists.into_iter().filter_map(|a| a.name).collect(),
            album_title: self
                .album
                .as_ref()
                .and_then(|a| a.name.clone())
                .unwrap_or_default(),
            duration_ms: self.duration_ms,
            artwork_urls: self
                .album
                .map(|a| a.images.into_iter().map(|i| i.url).collect())
                .unwrap_or_default(),
            explicit: self.explicit,
            external_url: self.external_urls.and_then(|u| u.spotify),
            is_playable: self.is_playable,
            linked_from_uri: self.linked_from.and_then(|l| l.uri),
            restrictions_reason: self.restrictions.and_then(|r| r.reason),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SearchTracks {
    #[serde(default)]
    pub items: Vec<Track>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SearchResponse {
    #[serde(default)]
    pub tracks: Option<SearchTracks>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Device {
    pub id: Option<String>,
    pub name: String,
    pub r#type: String,
    pub is_active: bool,
    pub is_private_session: bool,
    pub is_restricted: bool,
    #[serde(default)]
    pub volume_percent: Option<u8>,
    #[serde(default)]
    pub supports_volume: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DevicesResponse {
    #[serde(default)]
    pub devices: Vec<Device>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlaybackContext {
    #[serde(default)]
    pub uri: Option<String>,
    #[serde(default)]
    pub r#type: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CurrentPlayback {
    #[serde(default)]
    pub device: Option<Device>,
    #[serde(default)]
    pub repeat_state: Option<String>,
    #[serde(default)]
    pub shuffle_state: Option<bool>,
    #[serde(default)]
    pub context: Option<PlaybackContext>,
    #[serde(default)]
    pub progress_ms: Option<u64>,
    #[serde(default)]
    pub is_playing: bool,
    #[serde(default)]
    pub item: Option<Track>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlaylistTrackItem {
    #[serde(default)]
    pub track: Option<Track>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Paging<T> {
    #[serde(default = "Vec::new")]
    pub items: Vec<T>,
    #[serde(default)]
    pub next: Option<String>,
    #[serde(default)]
    pub total: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlaylistRef {
    pub id: String,
    pub name: String,
    pub uri: String,
    #[serde(default)]
    pub images: Vec<Image>,
    #[serde(default)]
    pub external_urls: Option<ExternalUrls>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SavedTrack {
    pub track: Track,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AlbumRefFull {
    pub id: String,
    pub name: String,
    pub uri: String,
    #[serde(default)]
    pub images: Vec<Image>,
    #[serde(default)]
    pub artists: Vec<ArtistRef>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SavedAlbum {
    pub album: AlbumRefFull,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QueueResponse {
    #[serde(default)]
    pub currently_playing: Option<Track>,
    #[serde(default)]
    pub queue: Vec<Track>,
}

/// Spotify's repeat modes accepted by `PUT /me/player/repeat`.
#[derive(Debug, Clone, Copy)]
pub enum RepeatState {
    Off,
    Context,
    Track,
}

impl RepeatState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Context => "context",
            Self::Track => "track",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct PlayBody<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    context_uri: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    uris: Option<Vec<&'a str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    offset: Option<PlayOffset<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    position_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
struct PlayOffset<'a> {
    uri: &'a str,
}

#[derive(Debug, Clone, Serialize)]
struct TransferBody<'a> {
    device_ids: Vec<&'a str>,
    play: bool,
}

pub struct WebApi<S: TokenStore> {
    pub auth: Arc<AuthCore<S>>,
    pub market: Option<String>,
    pub base: String,
}

impl<S: TokenStore> WebApi<S> {
    pub fn new(auth: Arc<AuthCore<S>>, market: Option<String>) -> Self {
        Self {
            auth,
            market,
            base: API_BASE.to_string(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    async fn bearer(&self) -> Result<String> {
        let t = self.auth.refresh_if_needed().await?;
        Ok(t.access_token)
    }

    /// Send a request and apply error mapping plus refresh-and-retry on 401.
    async fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, String)],
        json_body: Option<&serde_json::Value>,
    ) -> Result<reqwest::Response> {
        let mut attempt = 0u8;
        loop {
            attempt += 1;
            let token = self.bearer().await?;
            let mut req = self
                .auth
                .http
                .request(method.clone(), self.url(path))
                .bearer_auth(&token);
            if !query.is_empty() {
                req = req.query(query);
            }
            if let Some(b) = json_body {
                req = req.json(b);
            }
            let resp = req.send().await?;
            let status = resp.status().as_u16();
            if status == 401 && attempt == 1 {
                // Forced refresh and retry once.
                let _ = self.auth.force_refresh().await?;
                continue;
            }
            if (200..300).contains(&status) {
                return Ok(resp);
            }
            let retry_after = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok());
            let body = resp.text().await.unwrap_or_default();
            return Err(classify_http(status, &body, retry_after));
        }
    }

    async fn send_json<T: serde::de::DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, String)],
        json_body: Option<&serde_json::Value>,
    ) -> Result<T> {
        let resp = self.send(method, path, query, json_body).await?;
        let body = resp.text().await?;
        if body.trim().is_empty() {
            return Err(SpotifyError::MalformedResponse("empty body".into()));
        }
        Ok(serde_json::from_str(&body)?)
    }

    async fn send_optional_json<T: serde::de::DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, String)],
        json_body: Option<&serde_json::Value>,
    ) -> Result<Option<T>> {
        let resp = self.send(method, path, query, json_body).await?;
        if resp.status().as_u16() == 204 {
            return Ok(None);
        }
        let body = resp.text().await?;
        if body.trim().is_empty() {
            return Ok(None);
        }
        Ok(Some(serde_json::from_str(&body)?))
    }

    // ------------- endpoints -------------

    pub async fn current_user_profile(&self) -> Result<UserProfile> {
        self.send_json(reqwest::Method::GET, "/me", &[], None).await
    }

    pub async fn search(&self, query: &str) -> Result<SearchResponse> {
        let mut q = vec![
            ("q", query.to_string()),
            ("type", "track,album,artist,playlist".to_string()),
            ("limit", "20".to_string()),
        ];
        if let Some(m) = &self.market {
            if !m.is_empty() {
                q.push(("market", m.clone()));
            }
        }
        self.send_json(reqwest::Method::GET, "/search", &q, None)
            .await
    }

    pub async fn current_playback(&self) -> Result<Option<CurrentPlayback>> {
        self.send_optional_json(reqwest::Method::GET, "/me/player", &[], None)
            .await
    }

    pub async fn currently_playing(&self) -> Result<Option<CurrentPlayback>> {
        self.send_optional_json(
            reqwest::Method::GET,
            "/me/player/currently-playing",
            &[],
            None,
        )
        .await
    }

    pub async fn available_devices(&self) -> Result<Vec<Device>> {
        let r: DevicesResponse = self
            .send_json(reqwest::Method::GET, "/me/player/devices", &[], None)
            .await?;
        Ok(r.devices)
    }

    pub async fn transfer_playback(&self, device_id: &str, play: bool) -> Result<()> {
        let body = serde_json::to_value(TransferBody {
            device_ids: vec![device_id],
            play,
        })?;
        self.send(reqwest::Method::PUT, "/me/player", &[], Some(&body))
            .await?;
        Ok(())
    }

    pub async fn play(
        &self,
        device_id: Option<&str>,
        context_uri: Option<&str>,
        uris: Option<Vec<&str>>,
        offset_uri: Option<&str>,
        position_ms: Option<u64>,
    ) -> Result<()> {
        let body = PlayBody {
            context_uri,
            uris,
            offset: offset_uri.map(|u| PlayOffset { uri: u }),
            position_ms,
        };
        let mut q = vec![];
        if let Some(d) = device_id {
            q.push(("device_id", d.to_string()));
        }
        let json = serde_json::to_value(body)?;
        self.send(reqwest::Method::PUT, "/me/player/play", &q, Some(&json))
            .await?;
        Ok(())
    }

    pub async fn pause(&self, device_id: Option<&str>) -> Result<()> {
        let q = device_id
            .map(|d| vec![("device_id", d.to_string())])
            .unwrap_or_default();
        self.send(reqwest::Method::PUT, "/me/player/pause", &q, None)
            .await?;
        Ok(())
    }

    pub async fn next(&self, device_id: Option<&str>) -> Result<()> {
        let q = device_id
            .map(|d| vec![("device_id", d.to_string())])
            .unwrap_or_default();
        self.send(reqwest::Method::POST, "/me/player/next", &q, None)
            .await?;
        Ok(())
    }

    pub async fn previous(&self, device_id: Option<&str>) -> Result<()> {
        let q = device_id
            .map(|d| vec![("device_id", d.to_string())])
            .unwrap_or_default();
        self.send(reqwest::Method::POST, "/me/player/previous", &q, None)
            .await?;
        Ok(())
    }

    pub async fn seek(&self, position_ms: u64, device_id: Option<&str>) -> Result<()> {
        let mut q = vec![("position_ms", position_ms.to_string())];
        if let Some(d) = device_id {
            q.push(("device_id", d.to_string()));
        }
        self.send(reqwest::Method::PUT, "/me/player/seek", &q, None)
            .await?;
        Ok(())
    }

    pub async fn set_volume(&self, volume_percent: u8, device_id: Option<&str>) -> Result<()> {
        let mut q = vec![("volume_percent", volume_percent.min(100).to_string())];
        if let Some(d) = device_id {
            q.push(("device_id", d.to_string()));
        }
        self.send(reqwest::Method::PUT, "/me/player/volume", &q, None)
            .await?;
        Ok(())
    }

    pub async fn set_shuffle(&self, enabled: bool, device_id: Option<&str>) -> Result<()> {
        let mut q = vec![("state", enabled.to_string())];
        if let Some(d) = device_id {
            q.push(("device_id", d.to_string()));
        }
        self.send(reqwest::Method::PUT, "/me/player/shuffle", &q, None)
            .await?;
        Ok(())
    }

    pub async fn set_repeat(&self, state: RepeatState, device_id: Option<&str>) -> Result<()> {
        let mut q = vec![("state", state.as_str().to_string())];
        if let Some(d) = device_id {
            q.push(("device_id", d.to_string()));
        }
        self.send(reqwest::Method::PUT, "/me/player/repeat", &q, None)
            .await?;
        Ok(())
    }

    pub async fn get_queue(&self) -> Result<QueueResponse> {
        self.send_json(reqwest::Method::GET, "/me/player/queue", &[], None)
            .await
    }

    pub async fn add_to_queue(&self, uri: &str, device_id: Option<&str>) -> Result<()> {
        let mut q = vec![("uri", uri.to_string())];
        if let Some(d) = device_id {
            q.push(("device_id", d.to_string()));
        }
        self.send(reqwest::Method::POST, "/me/player/queue", &q, None)
            .await?;
        Ok(())
    }

    pub async fn current_user_playlists(&self) -> Result<Paging<PlaylistRef>> {
        self.send_json(
            reqwest::Method::GET,
            "/me/playlists",
            &[("limit", "50".to_string())],
            None,
        )
        .await
    }

    pub async fn playlist(&self, playlist_id: &str) -> Result<PlaylistRef> {
        let path = format!("/playlists/{}", playlist_id);
        self.send_json(reqwest::Method::GET, &path, &[], None).await
    }

    pub async fn playlist_items(&self, playlist_id: &str) -> Result<Paging<PlaylistTrackItem>> {
        let path = format!("/playlists/{}/tracks", playlist_id);
        self.send_json(
            reqwest::Method::GET,
            &path,
            &[("limit", "100".to_string())],
            None,
        )
        .await
    }

    /// `GET /me/tracks`. Uses the current generic library endpoint.
    pub async fn saved_tracks(&self) -> Result<Paging<SavedTrack>> {
        self.send_json(
            reqwest::Method::GET,
            "/me/tracks",
            &[("limit", "50".to_string())],
            None,
        )
        .await
    }

    /// `GET /me/albums`. Uses the current generic library endpoint.
    pub async fn saved_albums(&self) -> Result<Paging<SavedAlbum>> {
        self.send_json(
            reqwest::Method::GET,
            "/me/albums",
            &[("limit", "50".to_string())],
            None,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_search_track_basic() {
        let json = r#"{
          "tracks": {
            "items": [{
              "id": "T1", "uri": "spotify:track:T1", "name": "Hello",
              "duration_ms": 1234, "explicit": false,
              "artists": [{"name": "Adele"}],
              "album": {"name": "25", "images": [{"url": "https://i/1"}]},
              "external_urls": {"spotify": "https://open.spotify.com/track/T1"}
            }]
          }
        }"#;
        let v: SearchResponse = serde_json::from_str(json).unwrap();
        let tracks = v.tracks.unwrap();
        assert_eq!(tracks.items.len(), 1);
        let md = tracks.items[0].clone().into_metadata();
        assert_eq!(md.title, "Hello");
        assert_eq!(md.artist_names, vec!["Adele".to_string()]);
        assert_eq!(md.album_title, "25");
        assert_eq!(md.duration_ms, 1234);
        assert_eq!(md.artwork_urls, vec!["https://i/1".to_string()]);
    }

    #[test]
    fn parse_devices() {
        let json = r#"{"devices":[{
          "id":"D1","name":"Phone","type":"Smartphone",
          "is_active":true,"is_private_session":false,"is_restricted":false,
          "volume_percent":80
        }]}"#;
        let v: DevicesResponse = serde_json::from_str(json).unwrap();
        assert_eq!(v.devices.len(), 1);
        assert_eq!(v.devices[0].name, "Phone");
        assert!(v.devices[0].is_active);
    }

    #[test]
    fn parse_current_playback_missing_fields_does_not_panic() {
        let json = r#"{"is_playing":false}"#;
        let v: CurrentPlayback = serde_json::from_str(json).unwrap();
        assert!(!v.is_playing);
        assert!(v.item.is_none());
        assert!(v.device.is_none());
    }

    #[test]
    fn parse_playlist_paging() {
        let json = r#"{"items":[{"id":"P","name":"My","uri":"spotify:playlist:P","images":[]}],"next":null,"total":1}"#;
        let v: Paging<PlaylistRef> = serde_json::from_str(json).unwrap();
        assert_eq!(v.items.len(), 1);
        assert_eq!(v.items[0].id, "P");
    }

    #[test]
    fn track_with_linked_from_and_restrictions() {
        let json = r#"{
          "id":"T","uri":"spotify:track:T","name":"X",
          "duration_ms":0,"explicit":false,
          "artists":[],
          "linked_from":{"id":"OLD","uri":"spotify:track:OLD"},
          "restrictions":{"reason":"market"},
          "is_playable": false
        }"#;
        let t: Track = serde_json::from_str(json).unwrap();
        let md = t.into_metadata();
        assert_eq!(md.linked_from_uri.as_deref(), Some("spotify:track:OLD"));
        assert_eq!(md.restrictions_reason.as_deref(), Some("market"));
        assert_eq!(md.is_playable, Some(false));
    }
}
