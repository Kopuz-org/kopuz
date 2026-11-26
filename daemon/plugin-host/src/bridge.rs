//! What an integration may ask of kopuz itself, beyond the network. The host
//! supplies it; this crate only checks the manifest allows each call.

use std::future::Future;
use std::pin::Pin;

use crate::wit::{library, playback};

/// A plain-text reason, shown to the plugin and logged.
pub type Answer<T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send>>;

pub trait Bridge: Send + Sync + 'static {
    fn control(&self, command: playback::Command) -> Answer<()>;
    fn queue(&self, keys: Vec<String>, mode: playback::QueueMode) -> Answer<()>;

    fn search(&self, query: String, limit: u32) -> Answer<Vec<library::Track>>;
    fn tracks(&self, keys: Vec<String>) -> Answer<Vec<library::Track>>;
    fn album_tracks(&self, album_id: String) -> Answer<Vec<library::Track>>;
    fn artist_tracks(&self, artist_key: String) -> Answer<Vec<library::Track>>;
    fn tracks_page(&self, offset: u32, limit: u32) -> Answer<library::TrackPage>;
    fn playlists(&self) -> Answer<Vec<library::Playlist>>;
    fn favorites(&self) -> Answer<Vec<String>>;
}

/// The most a single library call hands a plugin.
pub const MAX_PAGE: u32 = 500;
