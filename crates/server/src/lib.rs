//! Streaming server backends for Kopuz: Jellyfin, Subsonic/Navidrome,
//! Nextcloud, YouTube Music, and the local download queue manager, plus the
//! stream sources and lyric providers they are read through.

pub mod applemusic;
pub mod cookies;
pub mod cover;
pub mod download_queue;
pub mod jellyfin;
pub mod lyrics;
pub mod musicbrainz;
pub mod nextcloud;
pub mod playback_ref;
pub mod provider;
pub mod server_ops;
pub mod soundcloud;
pub mod source;
pub mod spotify;
pub mod stream;
pub mod subsonic;
pub mod sync;
pub mod ytmusic;

pub use download_queue::{DownloadItem, DownloadProgress, DownloadQueue, DownloadStatus};
