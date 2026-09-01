//! DownloadsService: offline-caching server tracks, ported from the app's
//! `pages/server/cache.rs` policy (hashed filenames that cannot escape the
//! cache, partial-file publication, guarded deletes) with the queue and
//! progress reporting moved onto the job runner.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use api::{ApiError, ApiEvent, JobKind, JobRef, Table};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use utils::playback_ref::PlaybackItemRef;

use crate::config_service::ConfigService;
use crate::jobs::{JobCtx, JobRunner};
use crate::session::SessionHandle;

pub struct DownloadsService {
    db: db::Db,
    session: SessionHandle,
    config: Arc<ConfigService>,
    cache_dir: PathBuf,
}

fn safe_extension(extension: &str) -> Result<&str, ApiError> {
    let extension = extension.trim_start_matches('.');
    if extension.is_empty()
        || extension.len() > 8
        || !extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        return Err(ApiError::invalid_input(format!(
            "invalid download extension: {extension:?}"
        )));
    }
    Ok(extension)
}

fn content_type_to_ext(content_type: &str) -> Option<&'static str> {
    let content_type = content_type.split(';').next().unwrap_or("").trim();
    match content_type {
        "audio/mpeg" => Some("mp3"),
        "audio/mp4" | "audio/m4a" | "audio/x-m4a" => Some("m4a"),
        "audio/ogg" | "application/ogg" => Some("ogg"),
        "audio/webm" | "video/webm" => Some("webm"),
        "audio/flac" | "audio/x-flac" => Some("flac"),
        "audio/wav" | "audio/x-wav" => Some("wav"),
        "audio/aac" => Some("aac"),
        _ => None,
    }
}

impl DownloadsService {
    pub fn new(
        db: db::Db,
        session: SessionHandle,
        config: Arc<ConfigService>,
        cache_dir: PathBuf,
    ) -> Arc<Self> {
        Arc::new(Self {
            db,
            session,
            config,
            cache_dir,
        })
    }

    /// Item ids with a registered offline copy.
    pub async fn list(&self) -> Vec<String> {
        let config = self.session.config_watch().borrow().clone();
        let mut ids: Vec<String> = config.offline_tracks.keys().cloned().collect();
        ids.sort();
        ids
    }

    /// Derive a cache-local filename without trusting the remote item id.
    fn cache_file_path(&self, item_id: &str, extension: &str) -> Result<PathBuf, ApiError> {
        let extension = safe_extension(extension)?;
        let digest = Sha256::digest(item_id.as_bytes());
        let mut filename = String::with_capacity(digest.len() * 2 + extension.len() + 1);
        for byte in digest {
            let _ = write!(filename, "{byte:02x}");
        }
        filename.push('.');
        filename.push_str(extension);
        Ok(self.cache_dir.join(filename))
    }

    /// Delete only files whose parent resolves to the offline cache.
    fn remove_cache_file(&self, path: &Path) -> std::io::Result<bool> {
        let cache =
            std::fs::canonicalize(&self.cache_dir).unwrap_or_else(|_| self.cache_dir.clone());
        let Some(parent) = path.parent() else {
            return Ok(false);
        };
        let parent = std::fs::canonicalize(parent).unwrap_or_else(|_| parent.to_owned());
        if parent != cache {
            return Ok(false);
        }
        match std::fs::remove_file(path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    async fn register(&self, item_id: &str, path: Option<String>) -> Result<(), ApiError> {
        let updated = self.config.set_offline_track(item_id, path).await?;
        self.session
            .set_config(updated, vec!["offline_tracks".to_string()]);
        self.session.emit_event(ApiEvent::LibraryInvalidated {
            table: Table::Tracks,
            generation: 0,
        });
        Ok(())
    }

    pub fn spawn_download(
        self: &Arc<Self>,
        runner: &JobRunner,
        keys: Vec<String>,
    ) -> Result<JobRef, ApiError> {
        if keys.is_empty() {
            return Err(ApiError::invalid_input("no track keys to download"));
        }
        let service = self.clone();
        runner.start(JobKind::Download, move |ctx| async move {
            service.run_downloads(&ctx, keys).await
        })
    }

    async fn run_downloads(&self, ctx: &JobCtx, keys: Vec<String>) -> Result<(), ApiError> {
        let config = self.session.config_watch().borrow().clone();
        let source: server::source::ActiveSource =
            Arc::from(server::source::active(self.db.clone(), &config));
        let total = keys.len() as u64;
        let mut failed: Vec<String> = Vec::new();
        for (index, key) in keys.iter().enumerate() {
            if ctx.cancelled() {
                return Ok(());
            }
            ctx.progress(
                "downloading",
                Some(index as u64),
                Some(total),
                Some(key.clone()),
            );
            if let Err(error) = self.download_one(&source, &config, key).await {
                tracing::warn!(%error, %key, "download failed");
                failed.push(key.clone());
            }
        }
        ctx.progress("done", Some(total), Some(total), None);
        if failed.is_empty() {
            Ok(())
        } else {
            Err(ApiError::internal(format!(
                "{} of {} downloads failed: {}",
                failed.len(),
                total,
                failed.join(", ")
            )))
        }
    }

    async fn download_one(
        &self,
        source: &server::source::ActiveSource,
        config: &config::AppConfig,
        key: &str,
    ) -> Result<(), ApiError> {
        let uid = self
            .db
            .tracks_by_keys(&config.active_source, &[key.to_string()])
            .await
            .map_err(|error| ApiError::internal(format!("database error: {error}")))?
            .into_iter()
            .next()
            .map(|track| track.id.uid())
            .unwrap_or_else(|| key.to_string());
        let item_ref = PlaybackItemRef::parse(&uid);
        if !item_ref.is_server() {
            return Err(ApiError::invalid_input(
                "only server tracks can be cached offline",
            ));
        }
        let item_id = item_ref
            .primary_id()
            .ok_or_else(|| ApiError::invalid_input("track key has no item id"))?
            .to_string();

        if let Some(existing) = config.offline_tracks.get(&item_id)
            && Path::new(existing).is_file()
        {
            return Ok(());
        }

        let path = match source.download_track(&item_id, None).await {
            Ok(bytes) => {
                let path = self.cache_file_path(&item_id, "m4a")?;
                self.publish_bytes(&path, &bytes).await?
            }
            Err(server::source::SourceError::Unsupported(_)) => {
                let info = source.resolve_stream(&item_id).await.map_err(|error| {
                    ApiError::internal(format!("stream resolve failed: {error}"))
                })?;
                let hint = info
                    .format
                    .map(|(format, _)| format.extension())
                    .unwrap_or("m4a");
                self.fetch_to_cache(&item_id, &info.url, hint).await?
            }
            Err(error) => {
                return Err(ApiError::internal(format!("download failed: {error}")));
            }
        };
        self.register(&item_id, Some(path.to_string_lossy().into_owned()))
            .await
    }

    async fn publish_bytes(&self, path: &Path, bytes: &[u8]) -> Result<PathBuf, ApiError> {
        let io = |error: std::io::Error| ApiError::internal(format!("offline cache io: {error}"));
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(io)?;
        }
        tokio::fs::write(path, bytes).await.map_err(io)?;
        Ok(path.to_path_buf())
    }

    async fn fetch_to_cache(
        &self,
        item_id: &str,
        url: &str,
        extension_hint: &str,
    ) -> Result<PathBuf, ApiError> {
        let http = |error: reqwest::Error| {
            ApiError::internal(format!("download request failed: {}", error.without_url()))
        };
        let mut response = reqwest::get(url).await.map_err(http)?;
        response.error_for_status_ref().map_err(http)?;

        let extension = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(content_type_to_ext)
            .unwrap_or(extension_hint);
        let final_path = self.cache_file_path(item_id, extension)?;
        let io = |error: std::io::Error| ApiError::internal(format!("offline cache io: {error}"));
        if let Some(parent) = final_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(io)?;
        }
        let partial_path = final_path.with_extension(format!(
            "{}.part-{}",
            safe_extension(extension)?,
            uuid::Uuid::new_v4()
        ));
        let result: Result<(), ApiError> = async {
            let file = tokio::fs::File::create(&partial_path).await.map_err(io)?;
            let mut writer = tokio::io::BufWriter::with_capacity(256 * 1024, file);
            while let Some(chunk) = response.chunk().await.map_err(http)? {
                writer.write_all(&chunk).await.map_err(io)?;
            }
            writer.flush().await.map_err(io)
        }
        .await;
        if let Err(error) = result {
            let _ = tokio::fs::remove_file(&partial_path).await;
            return Err(error);
        }
        tokio::fs::rename(&partial_path, &final_path)
            .await
            .map_err(io)?;
        Ok(final_path)
    }

    pub async fn remove(&self, key: &str) -> Result<(), ApiError> {
        let item_ref = PlaybackItemRef::parse(key);
        let item_id = item_ref.primary_id().unwrap_or(key).to_string();
        let config = self.session.config_watch().borrow().clone();
        let Some(path) = config.offline_tracks.get(&item_id).cloned() else {
            return Err(ApiError::not_found("no offline copy for this track"));
        };
        match self.remove_cache_file(Path::new(&path)) {
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(%error, %path, "offline file removal failed; unregistering anyway");
            }
        }
        self.register(&item_id, None).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashed_filenames_cannot_escape_the_cache() {
        let extension = safe_extension("mp3").expect("valid");
        assert_eq!(extension, "mp3");
        assert!(safe_extension("../../etc").is_err());
        assert!(safe_extension("audio").is_ok());
        assert!(safe_extension("").is_err());
    }

    #[test]
    fn content_types_map_to_extensions() {
        assert_eq!(content_type_to_ext("audio/mpeg"), Some("mp3"));
        assert_eq!(
            content_type_to_ext("audio/mp4; charset=binary"),
            Some("m4a")
        );
        assert_eq!(content_type_to_ext("text/html"), None);
    }
}
