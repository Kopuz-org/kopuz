//! Streaming provider trait and shared internal types.
//!
//! The trait is intentionally backend-agnostic. The Spotify provider exposes
//! two implementations: `SpotifyConnectBackend` and `SpotifyWebPlaybackBackend`.

use crate::error::Result;
use async_trait::async_trait;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepeatMode {
    Off,
    One,
    All,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    pub kind: SearchKind,
    pub id: String,
    pub uri: String,
    pub title: String,
    pub subtitle: String,
    pub artwork_url: Option<String>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchKind {
    Track,
    Album,
    Artist,
    Playlist,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistSummary {
    pub id: String,
    pub uri: String,
    pub name: String,
    pub artwork_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackSummary {
    pub id: String,
    pub uri: String,
    pub title: String,
    pub artists: Vec<String>,
    pub album: String,
    pub duration_ms: u64,
    pub artwork_url: Option<String>,
    pub explicit: bool,
}

#[derive(Debug, Clone)]
pub struct PlaybackDevice {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub is_active: bool,
    pub is_restricted: bool,
    pub volume_percent: Option<u8>,
}

#[derive(Debug, Clone)]
pub struct PlaybackState {
    pub is_playing: bool,
    pub track: Option<TrackSummary>,
    pub progress_ms: Option<u64>,
    pub device: Option<PlaybackDevice>,
    pub shuffle: bool,
    pub repeat: RepeatMode,
}

#[async_trait]
pub trait StreamingProvider: Send + Sync {
    async fn login(&self) -> Result<()>;
    async fn logout(&self) -> Result<()>;
    async fn is_logged_in(&self) -> Result<bool>;

    async fn search(&self, query: &str) -> Result<Vec<SearchResult>>;
    async fn user_playlists(&self) -> Result<Vec<PlaylistSummary>>;
    async fn playlist_tracks(&self, playlist_id: &str) -> Result<Vec<TrackSummary>>;
    async fn saved_tracks(&self) -> Result<Vec<TrackSummary>>;

    async fn devices(&self) -> Result<Vec<PlaybackDevice>>;
    async fn select_device(&self, device_id: &str) -> Result<()>;

    async fn play_uri(&self, uri: &str) -> Result<()>;
    async fn play_context(&self, context_uri: &str, offset_uri: Option<&str>) -> Result<()>;
    async fn pause(&self) -> Result<()>;
    async fn resume(&self) -> Result<()>;
    async fn stop(&self) -> Result<()>;
    async fn next(&self) -> Result<()>;
    async fn previous(&self) -> Result<()>;
    async fn seek(&self, position_ms: u64) -> Result<()>;
    async fn set_volume(&self, volume_percent: u8) -> Result<()>;
    async fn set_shuffle(&self, enabled: bool) -> Result<()>;
    async fn set_repeat(&self, mode: RepeatMode) -> Result<()>;
    async fn queue(&self, uri: &str) -> Result<()>;
    async fn current_state(&self) -> Result<Option<PlaybackState>>;
}

pub(crate) fn repeat_to_api(mode: RepeatMode) -> crate::web_api::RepeatState {
    match mode {
        RepeatMode::Off => crate::web_api::RepeatState::Off,
        RepeatMode::One => crate::web_api::RepeatState::Track,
        RepeatMode::All => crate::web_api::RepeatState::Context,
    }
}

pub(crate) fn repeat_from_api(s: Option<&str>) -> RepeatMode {
    match s {
        Some("track") => RepeatMode::One,
        Some("context") => RepeatMode::All,
        _ => RepeatMode::Off,
    }
}

pub(crate) fn track_to_summary(t: crate::web_api::Track) -> TrackSummary {
    let md = t.into_metadata();
    TrackSummary {
        id: md.id,
        uri: md.uri,
        title: md.title,
        artists: md.artist_names,
        album: md.album_title,
        duration_ms: md.duration_ms,
        artwork_url: md.artwork_urls.into_iter().next(),
        explicit: md.explicit,
    }
}

pub(crate) fn device_to_summary(d: crate::web_api::Device) -> PlaybackDevice {
    PlaybackDevice {
        id: d.id.unwrap_or_default(),
        name: d.name,
        kind: d.r#type,
        is_active: d.is_active,
        is_restricted: d.is_restricted,
        volume_percent: d.volume_percent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeat_round_trip() {
        assert_eq!(repeat_from_api(Some("track")), RepeatMode::One);
        assert_eq!(repeat_from_api(Some("context")), RepeatMode::All);
        assert_eq!(repeat_from_api(Some("off")), RepeatMode::Off);
        assert_eq!(repeat_from_api(None), RepeatMode::Off);
        assert_eq!(repeat_to_api(RepeatMode::One).as_str(), "track");
        assert_eq!(repeat_to_api(RepeatMode::All).as_str(), "context");
        assert_eq!(repeat_to_api(RepeatMode::Off).as_str(), "off");
    }
}
