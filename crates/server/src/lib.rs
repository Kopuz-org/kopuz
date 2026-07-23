//! Streaming server backends for Kopuz: Jellyfin, Subsonic/Navidrome,
//! YouTube Music, and the local download queue manager.

pub mod cookies;
pub mod cover;
pub mod download_queue;
pub mod jellyfin;
pub mod provider;
pub mod server_ops;
pub mod soundcloud;
pub mod source;
pub mod spotify;
pub mod subsonic;
pub mod sync;
pub mod ytmusic;

pub use download_queue::{DownloadItem, DownloadProgress, DownloadQueue, DownloadStatus};
