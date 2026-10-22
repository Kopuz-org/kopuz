//! Streaming server backends for Kopuz: Nextcloud, YouTube Music, the plugin
//! host that serves the rest (Jellyfin and Subsonic/Navidrome among them), and
//! the local download queue manager, plus the stream sources and lyric
//! providers they are read through.

pub mod applemusic;
pub mod cookies;
pub mod cover;
pub mod download_queue;
pub mod lyrics;
pub mod musicbrainz;
pub mod nextcloud;
pub mod playback_ref;
pub mod plugins;
pub mod provider;
pub mod server_ops;
pub mod smb;
pub mod soundcloud;
pub mod source;
pub mod sources;
pub mod spotify;
pub mod stream;
pub mod sync;
pub mod youtube_download;
pub mod ytmusic;

pub use download_queue::{DownloadItem, DownloadProgress, DownloadQueue, DownloadStatus};
