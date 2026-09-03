//! DownloadsService: offline-caching server tracks, ported from the app's
//! `pages/server/cache.rs` policy (hashed filenames that cannot escape the
//! cache, partial-file publication, guarded deletes) with the queue and
//! progress reporting moved onto the job runner.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use api::{
    ApiError, ApiEvent, DownloadItemState, DownloadItemStatus, ErrorCode, JobKind, JobRef, Table,
};
use futures_util::{StreamExt as _, stream};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use utils::playback_ref::PlaybackItemRef;

use crate::config_service::ConfigService;
use crate::jobs::{JobCtx, JobRunner};
use crate::session::SessionHandle;

/// Upper bound on a single read from the download stream; a server that
/// accepts the request and then stalls without closing the socket would
/// otherwise leave the job Running forever.
const CHUNK_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

pub struct DownloadsService {
    db: db::Db,
    session: SessionHandle,
    config: Arc<ConfigService>,
    cache_dir: PathBuf,
    statuses: Mutex<std::collections::BTreeMap<String, DownloadItemStatus>>,
    cancelled_items: Mutex<std::collections::HashSet<String>>,
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
            statuses: Mutex::new(std::collections::BTreeMap::new()),
            cancelled_items: Mutex::new(std::collections::HashSet::new()),
        })
    }

    /// Item ids with a registered offline copy.
    pub async fn list(&self) -> Vec<String> {
        let config = self.session.config_watch().borrow().clone();
        let mut ids: Vec<String> = config.offline_tracks.keys().cloned().collect();
        ids.sort();
        ids
    }

    pub fn statuses(&self) -> Vec<DownloadItemStatus> {
        self.statuses
            .lock()
            .map(|statuses| statuses.values().cloned().collect())
            .unwrap_or_default()
    }

    pub fn cancel_item(&self, key: &str) -> Result<(), ApiError> {
        let known = self
            .statuses
            .lock()
            .map(|statuses| statuses.contains_key(key))
            .unwrap_or(false);
        if !known {
            return Err(ApiError::not_found("download item not found"));
        }
        if let Ok(mut cancelled) = self.cancelled_items.lock() {
            cancelled.insert(key.to_string());
        }
        self.update_status(key, DownloadItemState::Cancelled, 0, None, None);
        Ok(())
    }

    fn item_cancelled(&self, key: &str) -> bool {
        self.cancelled_items
            .lock()
            .map(|cancelled| cancelled.contains(key))
            .unwrap_or(true)
    }

    fn update_status(
        &self,
        key: &str,
        state: DownloadItemState,
        bytes_done: u64,
        total_bytes: Option<u64>,
        error: Option<String>,
    ) {
        if let Ok(mut statuses) = self.statuses.lock() {
            let entry = statuses
                .entry(key.to_string())
                .or_insert_with(|| DownloadItemStatus {
                    key: key.to_string(),
                    ..Default::default()
                });
            entry.state = state;
            entry.bytes_done = bytes_done;
            if total_bytes.is_some() {
                entry.total_bytes = total_bytes;
            }
            entry.error = error;
        }
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
        if let Ok(mut cancelled) = self.cancelled_items.lock() {
            for key in &keys {
                cancelled.remove(key);
            }
        }
        for key in &keys {
            self.update_status(key, DownloadItemState::Queued, 0, None, None);
        }
        let service = self.clone();
        runner.start(JobKind::Download, move |ctx| async move {
            service.run_downloads(&ctx, keys).await
        })
    }

    async fn run_downloads(
        self: &Arc<Self>,
        ctx: &JobCtx,
        keys: Vec<String>,
    ) -> Result<(), ApiError> {
        let config = self.session.config_watch().borrow().clone();
        let source: server::source::ActiveSource =
            Arc::from(server::source::active(self.db.clone(), &config));
        let total = keys.len() as u64;
        let completed = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let failed: Vec<String> = stream::iter(keys)
            .map(|key| {
                let service = self.clone();
                let source = source.clone();
                let config = config.clone();
                let ctx = ctx.clone();
                let completed = completed.clone();
                async move {
                    if ctx.cancelled() || service.item_cancelled(&key) {
                        service.update_status(&key, DownloadItemState::Cancelled, 0, None, None);
                        return None;
                    }
                    service.update_status(&key, DownloadItemState::Downloading, 0, None, None);
                    let result = service.download_one(&source, &config, &key, &ctx).await;
                    let current = completed.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                    ctx.progress("downloading", Some(current), Some(total), Some(key.clone()));
                    match result {
                        Ok(()) if service.item_cancelled(&key) || ctx.cancelled() => {
                            let _ = service.remove(&key).await;
                            service.update_status(
                                &key,
                                DownloadItemState::Cancelled,
                                0,
                                None,
                                None,
                            );
                            None
                        }
                        Ok(()) => {
                            service.update_status(
                                &key,
                                DownloadItemState::Finished,
                                service
                                    .statuses
                                    .lock()
                                    .ok()
                                    .and_then(|statuses| {
                                        statuses.get(&key).map(|item| item.bytes_done)
                                    })
                                    .unwrap_or_default(),
                                None,
                                None,
                            );
                            None
                        }
                        Err(error) if service.item_cancelled(&key) || ctx.cancelled() => {
                            service.update_status(
                                &key,
                                DownloadItemState::Cancelled,
                                0,
                                None,
                                None,
                            );
                            let _ = error;
                            None
                        }
                        Err(error) => {
                            tracing::warn!(%error, %key, "download failed");
                            service.update_status(
                                &key,
                                DownloadItemState::Failed,
                                0,
                                None,
                                Some(error.to_string()),
                            );
                            Some(key)
                        }
                    }
                }
            })
            .buffer_unordered(4)
            .filter_map(|key| async move { key })
            .collect()
            .await;
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
        ctx: &JobCtx,
    ) -> Result<(), ApiError> {
        let track = self
            .db
            .tracks_by_keys(&config.active_source, &[key.to_string()])
            .await
            .map_err(|error| ApiError::internal(format!("database error: {error}")))?
            .into_iter()
            .next()
            .ok_or_else(|| ApiError::not_found("track not found"))?;
        let reader::TrackId::Server { item_id, .. } = track.id else {
            return Err(ApiError::invalid_input(
                "only server tracks can be cached offline",
            ));
        };

        if let Some(existing) = config.offline_tracks.get(&item_id)
            && Path::new(existing).is_file()
        {
            return Ok(());
        }

        let path = match source.download_track(&item_id, None).await {
            Ok(bytes) => {
                let path = self.cache_file_path(&item_id, "m4a")?;
                let path = self.publish_bytes(&path, &bytes).await?;
                self.update_status(
                    key,
                    DownloadItemState::Downloading,
                    bytes.len() as u64,
                    Some(bytes.len() as u64),
                    None,
                );
                path
            }
            Err(server::source::SourceError::Unsupported(_)) => {
                let info = source.resolve_stream(&item_id).await.map_err(|error| {
                    ApiError::internal(format!("stream resolve failed: {error}"))
                })?;
                let hint = info
                    .format
                    .map(|(format, _)| format.extension())
                    .unwrap_or("m4a");
                self.fetch_to_cache(
                    key,
                    &item_id,
                    &info.url,
                    hint,
                    info.user_agent.as_deref(),
                    info.content_length,
                    ctx,
                )
                .await?
            }
            Err(error) => {
                return Err(ApiError::internal(format!("download failed: {error}")));
            }
        };
        if ctx.cancelled() || self.item_cancelled(key) {
            let _ = self.remove_cache_file(&path);
            return Err(ApiError::new(ErrorCode::Conflict, "download cancelled"));
        }
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
        key: &str,
        item_id: &str,
        url: &str,
        extension_hint: &str,
        user_agent: Option<&str>,
        content_length: Option<u64>,
        ctx: &JobCtx,
    ) -> Result<PathBuf, ApiError> {
        let http = |error: reqwest::Error| {
            ApiError::internal(format!("download request failed: {}", error.without_url()))
        };
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(15))
            .tcp_nodelay(true)
            .build()
            .map_err(http)?;
        let final_path = self.cache_file_path(item_id, extension_hint)?;
        let io = |error: std::io::Error| ApiError::internal(format!("offline cache io: {error}"));
        if let Some(parent) = final_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(io)?;
        }
        let partial_path = final_path.with_extension(format!(
            "{}.part-{}",
            safe_extension(extension_hint)?,
            uuid::Uuid::new_v4()
        ));
        if let (Some(user_agent), Some(total)) = (user_agent, content_length) {
            let result: Result<(), ApiError> = async {
                const CHUNK: u64 = 512 * 1024;
                let file = tokio::fs::File::create(&partial_path).await.map_err(io)?;
                let mut writer = tokio::io::BufWriter::with_capacity(256 * 1024, file);
                let mut start = 0;
                while start < total {
                    if ctx.cancelled() || self.item_cancelled(key) {
                        return Err(ApiError::new(ErrorCode::Conflict, "download cancelled"));
                    }
                    let end = (start + CHUNK - 1).min(total - 1);
                    let response = tokio::time::timeout(
                        std::time::Duration::from_secs(60),
                        client
                            .get(url)
                            .header(reqwest::header::USER_AGENT, user_agent)
                            .header(reqwest::header::RANGE, format!("bytes={start}-{end}"))
                            .send(),
                    )
                    .await
                    .map_err(|_| ApiError::internal("download range request timed out"))?
                    .map_err(http)?;
                    if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
                        return Err(ApiError::internal(format!(
                            "download server ignored byte range {start}-{end}"
                        )));
                    }
                    let bytes = tokio::time::timeout(CHUNK_READ_TIMEOUT, response.bytes())
                        .await
                        .map_err(|_| ApiError::internal("download range read timed out"))?
                        .map_err(http)?;
                    let expected = end - start + 1;
                    if bytes.len() as u64 != expected {
                        return Err(ApiError::internal(format!(
                            "short download range {start}-{end}: received {} of {expected} bytes",
                            bytes.len()
                        )));
                    }
                    writer.write_all(&bytes).await.map_err(io)?;
                    start = end + 1;
                    self.update_status(
                        key,
                        DownloadItemState::Downloading,
                        start,
                        Some(total),
                        None,
                    );
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
            return Ok(final_path);
        }

        let mut request = client.get(url);
        if let Some(user_agent) = user_agent {
            request = request.header(reqwest::header::USER_AGENT, user_agent);
        }
        let mut response = tokio::time::timeout(std::time::Duration::from_secs(60), request.send())
            .await
            .map_err(|_| ApiError::internal("download request timed out"))?
            .map_err(http)?;
        response.error_for_status_ref().map_err(http)?;
        let total = response.content_length();
        let extension = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(content_type_to_ext)
            .unwrap_or(extension_hint);
        let resolved_path = self.cache_file_path(item_id, extension)?;
        let resolved_partial = resolved_path.with_extension(format!(
            "{}.part-{}",
            safe_extension(extension)?,
            uuid::Uuid::new_v4()
        ));
        let result: Result<(), ApiError> = async {
            let file = tokio::fs::File::create(&resolved_partial)
                .await
                .map_err(io)?;
            let mut writer = tokio::io::BufWriter::with_capacity(256 * 1024, file);
            let mut bytes_done = 0;
            // A stalled socket must not hang the job forever: bound every
            // chunk read, matching the old download-queue behavior.
            loop {
                if ctx.cancelled() || self.item_cancelled(key) {
                    return Err(ApiError::new(ErrorCode::Conflict, "download cancelled"));
                }
                let chunk = tokio::time::timeout(CHUNK_READ_TIMEOUT, response.chunk())
                    .await
                    .map_err(|_| ApiError::internal("download stalled: chunk read timed out"))?
                    .map_err(http)?;
                if ctx.cancelled() || self.item_cancelled(key) {
                    return Err(ApiError::new(ErrorCode::Conflict, "download cancelled"));
                }
                let Some(chunk) = chunk else {
                    break;
                };
                writer.write_all(&chunk).await.map_err(io)?;
                bytes_done += chunk.len() as u64;
                self.update_status(key, DownloadItemState::Downloading, bytes_done, total, None);
            }
            writer.flush().await.map_err(io)?;
            if ctx.cancelled() || self.item_cancelled(key) {
                return Err(ApiError::new(ErrorCode::Conflict, "download cancelled"));
            }
            Ok(())
        }
        .await;
        if let Err(error) = result {
            let _ = tokio::fs::remove_file(&resolved_partial).await;
            return Err(error);
        }
        tokio::fs::rename(&resolved_partial, &resolved_path)
            .await
            .map_err(io)?;
        Ok(resolved_path)
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
