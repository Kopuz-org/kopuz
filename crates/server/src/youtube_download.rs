//! Native YouTube Music search and stream resolution for the download tool.
//!
//! This deliberately reuses the same InnerTube, authentication, BotGuard, and
//! decipher paths as playback. The UI never shells out to a site downloader or
//! handles YouTube credentials itself.

use config::{AppConfig, MusicService};
use reader::Track;

use crate::ytmusic::{YtStreamInfo, search::music_search_tracks};

#[derive(Clone, Default)]
pub struct YoutubeDownloadClient {
    cookies: Option<String>,
}

impl YoutubeDownloadClient {
    /// Use the active YouTube Music session when one is available. The native
    /// endpoints also support anonymous search and public-track downloads, so
    /// this client remains useful while the local library is active.
    pub fn from_config(config: &AppConfig) -> Self {
        let cookies = config
            .server
            .as_ref()
            .filter(|server| server.service == MusicService::YtMusic)
            .and_then(|server| server.access_token.as_deref())
            .filter(|token| !token.is_empty())
            .map(str::to_owned);
        Self { cookies }
    }

    pub async fn search(&self, query: &str) -> Result<Vec<Track>, String> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        music_search_tracks(query, self.cookies.as_deref()).await
    }

    pub async fn resolve_stream(&self, video_id: &str) -> Result<YtStreamInfo, String> {
        let video_id = video_id.trim();
        if video_id.is_empty() {
            return Err("track has no YouTube video id".to_string());
        }
        crate::ytmusic::probe_stream(video_id, self.cookies.as_deref()).await
    }
}
