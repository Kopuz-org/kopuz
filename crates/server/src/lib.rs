//! Streaming server backends for Kopuz: Jellyfin, Subsonic/Navidrome,
//! YouTube Music, and the local download queue manager.

pub mod download_queue;
pub mod jellyfin;
pub mod provider;
pub mod server_ops;
pub mod subsonic;
pub mod ytmusic;

pub use download_queue::{DownloadItem, DownloadProgress, DownloadQueue, DownloadStatus};
