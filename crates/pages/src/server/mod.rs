pub mod discover;
pub mod download_manager;
pub mod subsonic_sync;

mod cache;

pub use cache::download_track_to_cache;

use config::{AppConfig, MusicService};
use dioxus::prelude::{ReadableExt, WritableExt};

pub fn build_download_url(item_id: &str, config: &AppConfig) -> Option<(String, &'static str)> {
    let server = config.server.as_ref()?;
    let quality = config.offline_quality;
    let ext = quality.file_extension();

    let url = match server.service {
        MusicService::Jellyfin => {
            let token = server.access_token.as_deref().unwrap_or("");
            match quality.jellyfin_bitrate_bps() {
                Some(bps) => format!(
                    "{}/Audio/{}/stream?audioBitRate={}&audioCodec=mp3&api_key={}",
                    server.url, item_id, bps, token
                ),
                None => format!(
                    "{}/Audio/{}/stream?static=true&api_key={}",
                    server.url, item_id, token
                ),
            }
        }
        MusicService::Subsonic | MusicService::Custom => {
            let username = server.user_id.as_deref()?;
            let password_or_token = server.access_token.as_deref()?;
            let resolved_password = ::server::provider::resolve_subsonic_secret(password_or_token)?;
            let kbps = quality.subsonic_max_bitrate_kbps();
            ::server::subsonic::stream_url_with_bitrate(
                &server.url,
                username,
                &resolved_password,
                item_id,
                Some(kbps),
            )
            .ok()?
        }
        MusicService::Nextcloud => {
            // No transcode to ask for: quality only picks the fallback
            // extension, corrected later from the response content type.
            let username = server.user_id.as_deref()?;
            let password = server.access_token.as_deref()?;
            // The URL itself carries the app password, so only the error is logged.
            ::server::nextcloud::stream_url(&server.url, username, password, item_id)
                .inspect_err(
                    |e| tracing::warn!(%item_id, error = %e, "nextcloud download URL build failed"),
                )
                .ok()?
        }
        MusicService::YtMusic
        | MusicService::SoundCloud
        | MusicService::AppleMusic
        | MusicService::Spotify => return None,
    };
    Some((url, ext))
}

pub(super) fn content_type_to_ext(content_type: &str) -> Option<&'static str> {
    let ct = content_type.split(';').next().unwrap_or("").trim();
    match ct {
        "audio/flac" | "audio/x-flac" => Some("flac"),
        "audio/mpeg" | "audio/mp3" => Some("mp3"),
        "audio/mp4" | "audio/x-m4a" | "video/mp4" => Some("m4a"),
        "audio/ogg" | "audio/opus" => Some("ogg"),
        "audio/webm" | "video/webm" => Some("webm"),
        "audio/x-matroska" | "audio/matroska" | "video/x-matroska" | "video/matroska" => {
            Some("mka")
        }
        "audio/aac" => Some("aac"),
        "audio/wav" | "audio/x-wav" => Some("wav"),
        "audio/aiff" | "audio/x-aiff" => Some("aiff"),
        _ => None,
    }
}

pub async fn download_tracks_batch(
    item_ids: Vec<String>,
    mut config: dioxus::prelude::Signal<AppConfig>,
) {
    for id in item_ids {
        let is_downloaded = if let Some(path_str) = config.read().offline_tracks.get(&id) {
            std::path::Path::new(path_str).exists()
        } else {
            false
        };
        if is_downloaded {
            continue;
        }
        let result = {
            let conf = config.read();
            build_download_url(&id, &conf)
        };
        if let Some((url, ext)) = result {
            match download_track_to_cache(&id, &url, ext).await {
                Ok(path) => {
                    config
                        .write()
                        .offline_tracks
                        .insert(id.clone(), path.to_string_lossy().into_owned());
                }
                Err(e) => tracing::warn!(%id, error = %e, "batch download failed"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::content_type_to_ext;

    #[test]
    fn maps_matroska_audio_to_mka() {
        assert_eq!(content_type_to_ext("audio/x-matroska"), Some("mka"));
        assert_eq!(content_type_to_ext("audio/matroska"), Some("mka"));
        assert_eq!(content_type_to_ext("video/x-matroska"), Some("mka"));
        assert_eq!(content_type_to_ext("video/matroska"), Some("mka"));
    }

    #[test]
    fn maps_matroska_with_codec_parameters() {
        assert_eq!(
            content_type_to_ext("audio/x-matroska; codecs=opus"),
            Some("mka")
        );
    }

    #[test]
    fn unknown_container_returns_none() {
        assert_eq!(content_type_to_ext("application/octet-stream"), None);
    }

    #[test]
    fn preserves_existing_mappings() {
        assert_eq!(content_type_to_ext("audio/mpeg"), Some("mp3"));
        assert_eq!(content_type_to_ext("audio/flac"), Some("flac"));
        assert_eq!(content_type_to_ext("audio/mp4"), Some("m4a"));
        assert_eq!(content_type_to_ext("audio/ogg"), Some("ogg"));
    }
}
