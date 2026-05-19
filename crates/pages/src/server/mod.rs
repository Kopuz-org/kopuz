pub mod activity;
pub mod download_manager;
pub mod album;
pub mod artist;
pub mod favorites;
pub mod home;
pub mod library;
pub mod playlists;
pub mod search;
pub mod subsonic_sync;
pub mod unsupported;

use config::{AppConfig, MusicService};
use dioxus::prelude::{ReadableExt, WritableExt};
use std::path::PathBuf;

pub(super) fn offline_cache_dir() -> PathBuf {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let base = directories::ProjectDirs::from("com", "temidaradev", "kopuz")
            .map(|dirs| dirs.cache_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("./cache"));
        let dir = base.join("offline_tracks");
        let _ = std::fs::create_dir_all(&dir);
        dir
    }
    #[cfg(target_arch = "wasm32")]
    PathBuf::from("./cache/offline_tracks")
}

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
            let client =
                ::server::subsonic::SubsonicClient::new(&server.url, username, &resolved_password);
            let kbps = quality.subsonic_max_bitrate_kbps();
            client.stream_url_with_bitrate(item_id, Some(kbps)).ok()?
        }
    };
    Some((url, ext))
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn content_type_to_ext(content_type: &str) -> Option<&'static str> {
    let ct = content_type.split(';').next().unwrap_or("").trim();
    match ct {
        "audio/flac" | "audio/x-flac" => Some("flac"),
        "audio/mpeg" | "audio/mp3" => Some("mp3"),
        "audio/mp4" | "audio/x-m4a" | "video/mp4" => Some("m4a"),
        "audio/ogg" | "audio/opus" => Some("ogg"),
        "audio/webm" | "video/webm" => Some("webm"),
        "audio/aac" => Some("aac"),
        "audio/wav" | "audio/x-wav" => Some("wav"),
        "audio/aiff" | "audio/x-aiff" => Some("aiff"),
        _ => None,
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn download_track_to_cache(
    item_id: &str,
    url: &str,
    ext_hint: &str,
) -> Result<PathBuf, String> {
    let response = reqwest::get(url)
        .await
        .map_err(|e| format!("Download failed: {e}"))?;

    let ext = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(content_type_to_ext)
        .unwrap_or(ext_hint);

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Failed to read response: {e}"))?;

    let dir = offline_cache_dir();
    let file_path = dir.join(format!("{item_id}.{ext}"));
    tokio::fs::write(&file_path, &bytes)
        .await
        .map_err(|e| format!("Failed to save file: {e}"))?;

    Ok(file_path)
}

#[cfg(not(target_arch = "wasm32"))]
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
                Err(e) => eprintln!("Batch download failed for {id}: {e}"),
            }
        }
    }
}
