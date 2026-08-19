use config::{AppConfig, MusicService};
use dioxus::core::spawn_forever;
use dioxus::prelude::*;
use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tracing::Instrument;

use ::server::source::SourceError;
pub use ::server::{DownloadItem, DownloadProgress, DownloadQueue, DownloadStatus};

thread_local! {
    static DOWNLOAD_PROGRESS: Cell<Option<Signal<DownloadProgress>>> = const { Cell::new(None) };
}

pub fn register_progress_signal(signal: Signal<DownloadProgress>) {
    DOWNLOAD_PROGRESS.with(|s| s.set(Some(signal)));
}

fn progress_signal() -> Option<Signal<DownloadProgress>> {
    DOWNLOAD_PROGRESS.with(|s| s.get())
}

fn publish_progress(item_id: &str, bytes_done: u64, bytes_delta: u64, elapsed_secs: f64) {
    let Some(mut p) = progress_signal() else {
        return;
    };
    let mut state = p.write();
    state.per_item.insert(item_id.to_string(), bytes_done);
    state.bytes_done_session += bytes_delta;
    state.session_elapsed_secs = elapsed_secs;
}

fn clear_progress(item_id: &str) {
    let Some(mut p) = progress_signal() else {
        return;
    };
    p.write().per_item.remove(item_id);
}

fn reset_progress_session() {
    let Some(mut p) = progress_signal() else {
        return;
    };
    let mut state = p.write();
    state.bytes_done_session = 0;
    state.session_elapsed_secs = 0.0;
}

pub fn queue_downloads(
    requests: Vec<(String, String, String)>,
    config: Signal<AppConfig>,
    mut queue: Signal<DownloadQueue>,
) {
    let mut added = false;
    let cancel_flag: Arc<AtomicBool>;
    {
        let mut q = queue.write();
        let conf = config.peek();
        let queued_ids: std::collections::HashSet<String> =
            q.items.iter().map(|i| i.id.clone()).collect();

        for (id, title, artist) in &requests {
            if conf.offline_tracks.contains_key(id) {
                continue;
            }
            if queued_ids.contains(id) {
                continue;
            }
            q.items.push(DownloadItem {
                id: id.clone(),
                title: title.clone(),
                artist: artist.clone(),
                status: DownloadStatus::Queued,
                bytes_done: 0,
                bytes_total: 0,
            });
            added = true;
        }

        if !added || q.is_running {
            return;
        }
        // Reset cancel flags only once we're sure we're actually starting
        // a fresh worker session. Replacing the Arc gives any still-living
        // worker from a prior cancelled session its own (still-set) flag
        // so it terminates instead of resuming on the new session's reset
        // signal.
        q.cancel_requested = false;
        q.cancel_flag = Arc::new(AtomicBool::new(false));
        cancel_flag = q.cancel_flag.clone();
        q.is_running = true;
    }

    reset_progress_session();

    let active_source = use_context::<Signal<::server::source::ActiveSource>>();
    let session_start = Instant::now();
    let session_span = tracing::info_span!("downloads.session");
    // spawn_forever: queue_downloads is called from page event handlers, and a
    // scope-tied spawn dies with the page — navigating away from the downloads
    // view cancelled the whole session mid-download (#327).
    spawn_forever(
        async move {
            tokio::join!(
                download_worker(
                    queue,
                    config,
                    active_source,
                    session_start,
                    cancel_flag.clone()
                ),
                download_worker(
                    queue,
                    config,
                    active_source,
                    session_start,
                    cancel_flag.clone()
                ),
                download_worker(
                    queue,
                    config,
                    active_source,
                    session_start,
                    cancel_flag.clone()
                ),
                download_worker(
                    queue,
                    config,
                    active_source,
                    session_start,
                    cancel_flag.clone()
                ),
            );

            let mut q = queue.write();
            q.is_running = false;
            q.cancel_requested = false;
        }
        .instrument(session_span),
    );
}

async fn download_worker(
    mut queue: Signal<DownloadQueue>,
    config: Signal<AppConfig>,
    active_source: Signal<::server::source::ActiveSource>,
    session_start: Instant,
    cancel_flag: Arc<AtomicBool>,
) {
    loop {
        if cancel_flag.load(Ordering::Relaxed) {
            return;
        }

        // Atomic claim: find + status flip in one write lock prevents two workers
        // grabbing the same id.
        let next_id = {
            let mut q = queue.write();
            let claimed = q
                .items
                .iter_mut()
                .find(|i| matches!(i.status, DownloadStatus::Queued));
            match claimed {
                Some(item) => {
                    item.status = DownloadStatus::Downloading;
                    Some(item.id.clone())
                }
                None => None,
            }
        };
        let Some(id) = next_id else {
            return;
        };

        if config.read().offline_tracks.contains_key(&id) {
            if let Some(item) = queue.write().items.iter_mut().find(|i| i.id == id) {
                item.status = DownloadStatus::Done;
            }
            continue;
        }

        let service = config.read().server.as_ref().map(|x| x.service);

        // Pinned for the whole download, so the file is registered against the
        // source that produced it even if the active one changes meanwhile.
        let source = active_source.peek().clone();

        // A source that can't express its audio as a URL hands back the bytes
        // instead. Asking the source rather than testing the service keeps this
        // out of the UI: everything else declines and falls through to the URL
        // path below.
        match download_from_source(&id, &source, &mut queue, &session_start, &cancel_flag).await {
            Err(SourceError::Unsupported(_)) => {}
            outcome => {
                let outcome = outcome.map_err(|e| e.to_string());
                finish_item(outcome, &id, &mut queue, &source, config).await;
                continue;
            }
        }

        let resolved: Option<(String, &'static str, Option<String>, Option<u64>)> =
            if matches!(service, Some(MusicService::YtMusic)) {
                let source = active_source.peek().clone();
                match source.resolve_stream(&id).await {
                    Ok(info) => Some((
                        info.url,
                        info.format.map(|(f, _)| f.extension()).unwrap_or_default(),
                        info.user_agent,
                        info.content_length,
                    )),
                    Err(e) => {
                        tracing::warn!(%id, error = %e, "YT download URL resolve failed");
                        None
                    }
                }
            } else {
                let conf = config.read();
                super::build_download_url(&id, &conf).map(|(u, ext)| (u, ext, None, None))
            };

        let (url, ext_hint, user_agent, content_length) = match resolved {
            Some(v) => v,
            None => {
                if let Some(item) = queue.write().items.iter_mut().find(|i| i.id == id) {
                    item.status = DownloadStatus::Failed;
                }
                continue;
            }
        };

        let outcome = download_with_progress(
            &id,
            &url,
            ext_hint,
            user_agent.as_deref(),
            content_length,
            &mut queue,
            &session_start,
            &cancel_flag,
        )
        .await;
        finish_item(outcome, &id, &mut queue, &source, config).await;
    }
}

/// Record a finished download: registry, queue status and progress.
///
/// `source` is the one the download actually ran against, passed in rather than
/// re-read here: a source switch while a track was downloading would otherwise
/// register the file against whichever backend happens to be active by the time
/// it lands.
async fn finish_item(
    outcome: Result<std::path::PathBuf, String>,
    id: &str,
    queue: &mut Signal<DownloadQueue>,
    source: &::server::source::ActiveSource,
    mut config: Signal<AppConfig>,
) {
    let outcome = match outcome {
        Ok(path) => {
            let path_str = path.to_string_lossy().into_owned();
            // Durable FIRST as a single json_set (the whole-config save per
            // completed song was the audio-stutter bug), then the in-memory
            // mirror for live reads. A file the registry doesn't know about is
            // one nothing will ever play or clean up, so a failure here means
            // the download failed — and the bytes go with it.
            match source.set_offline_track(id, Some(&path_str)).await {
                Ok(()) => {
                    config
                        .write()
                        .offline_tracks
                        .insert(id.to_string(), path_str);
                    Ok(())
                }
                Err(e) => {
                    let _ = tokio::fs::remove_file(&path).await;
                    Err(format!("register the downloaded file: {e}"))
                }
            }
        }
        Err(e) => Err(e),
    };

    match outcome {
        Ok(()) => {
            if let Some(item) = queue.write().items.iter_mut().find(|i| i.id == id) {
                item.status = DownloadStatus::Done;
            }
        }
        Err(e) => {
            tracing::error!(%id, error = %e, "download failed");
            if let Some(item) = queue.write().items.iter_mut().find(|i| i.id == id) {
                item.status = DownloadStatus::Failed;
            }
        }
    }
    clear_progress(id);
}

/// Download a track whose bytes only the source can produce.
///
/// Returns [`SourceError::Unsupported`] for every source that can express its
/// audio as a URL, which is the signal to use the URL path instead.
///
/// Nothing is chunked here and there is no partial file to clean up: the source
/// hands back a finished file and it is written once, at the end.
async fn download_from_source(
    id: &str,
    source: &::server::source::ActiveSource,
    queue: &mut Signal<DownloadQueue>,
    session_start: &Instant,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<std::path::PathBuf, SourceError> {
    // The source reports from whatever thread does its work — for Apple Music a
    // dedicated decrypt thread — and both the progress signal and the queue are
    // owned by the UI thread. So the callback only stores numbers, and the loop
    // below publishes them from here, where those signals actually exist.
    let done = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let total = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let progress = {
        let (done, total) = (done.clone(), total.clone());
        Arc::new(move |_from: u64, at: u64, of: Option<u64>| {
            done.store(at, Ordering::Relaxed);
            if let Some(of) = of {
                total.store(of, Ordering::Relaxed);
            }
        }) as ::utils::stream_buffer::BufferProgressCallback
    };

    let mut published = 0u64;
    let mut publish = |queue: &mut Signal<DownloadQueue>| {
        let (at, of) = (done.load(Ordering::Relaxed), total.load(Ordering::Relaxed));
        if of > 0 {
            let mut q = queue.write();
            if let Some(item) = q.items.iter_mut().find(|i| i.id == id) {
                item.bytes_total = of;
                item.bytes_done = at;
            }
        }
        if at > published {
            publish_progress(
                id,
                at,
                at - published,
                session_start.elapsed().as_secs_f64(),
            );
            published = at;
        }
    };

    let bytes = {
        let fetch = source.download_track(id, Some(progress));
        tokio::pin!(fetch);
        loop {
            tokio::select! {
                finished = &mut fetch => break finished?,
                _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                    publish(queue);
                    // Checked here rather than only after the download resolves:
                    // a source-provided download is one long await, so waiting
                    // for it to return means cancel does nothing until the track
                    // has finished anyway. Dropping `fetch` on the way out is
                    // what stops the transfer.
                    if cancel_flag.load(Ordering::Relaxed) {
                        return Err(SourceError::Backend("cancelled".to_string()));
                    }
                }
            }
        }
    };

    if cancel_flag.load(Ordering::Relaxed) {
        return Err(SourceError::Backend("cancelled".to_string()));
    }

    {
        let mut q = queue.write();
        if let Some(item) = q.items.iter_mut().find(|i| i.id == id) {
            item.bytes_total = bytes.len() as u64;
            item.bytes_done = bytes.len() as u64;
        }
    }

    let dir = super::offline_cache_dir();
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| SourceError::Backend(format!("create download dir: {e}")))?;
    // Decrypted Apple Music assets are MP4; nothing else reaches this path yet.
    let path = dir.join(format!("{id}.m4a"));
    tokio::fs::write(&path, &bytes)
        .await
        .map_err(|e| SourceError::Backend(format!("write download: {e}")))?;
    Ok(path)
}

pub fn delete_downloads(
    ids: Vec<String>,
    mut config: Signal<AppConfig>,
    mut queue: Signal<DownloadQueue>,
) {
    let active_source = use_context::<Signal<::server::source::ActiveSource>>();
    let mut conf = config.write();
    let mut q = queue.write();

    for id in ids {
        if let Some(path_str) = conf.offline_tracks.remove(&id) {
            let path = std::path::Path::new(&path_str);
            if path.exists() {
                let _ = std::fs::remove_file(path);
            }
        }
        let source = active_source.peek().clone();
        let spawn_id = id.clone();
        // The file is already deleted above — the DB row removal must
        // outlive the calling page or the registry points at nothing.
        spawn_forever(async move {
            let _ = source.set_offline_track(&spawn_id, None).await;
        });
        q.items.retain(|i| i.id != id);
    }
}

#[tracing::instrument(
    name = "download.track",
    skip(url, user_agent, queue, session_start, cancel_flag),
    fields(item_id = %item_id, content_length)
)]
async fn download_with_progress(
    item_id: &str,
    url: &str,
    ext_hint: &'static str,
    user_agent: Option<&str>,
    content_length: Option<u64>,
    queue: &mut Signal<DownloadQueue>,
    session_start: &Instant,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<std::path::PathBuf, String> {
    use tokio::io::AsyncWriteExt;

    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .tcp_nodelay(true)
        .build()
        .map_err(|e| format!("Client build error: {e}"))?;

    let dir = super::offline_cache_dir();
    let file_path_tentative = dir.join(format!("{item_id}.{ext_hint}"));

    // YT googlevideo URLs throttle single sequential GETs to ~1 MB/s; Range-chunking
    // sidesteps the throttle and saturates the link.
    if let (Some(ua), Some(total)) = (user_agent, content_length) {
        let ext = ext_hint;
        let file_path = dir.join(format!("{item_id}.{ext}"));
        let file = tokio::fs::File::create(&file_path)
            .await
            .map_err(|e| format!("Create file: {e}"))?;
        let mut writer = tokio::io::BufWriter::with_capacity(256 * 1024, file);

        {
            let mut q = queue.write();
            if let Some(item) = q.items.iter_mut().find(|i| i.id == item_id) {
                item.bytes_total = total;
            }
        }

        const CHUNK: u64 = 512 * 1024;
        const RANGE_TIMEOUT_SECS: u64 = 60;
        const UI_UPDATE_MS: u128 = 50;

        let mut start = 0u64;
        let mut bytes_done = 0u64;
        let mut last_update_at = Instant::now();
        let mut last_update_bytes = 0u64;
        let mut first_update_done = false;

        while start < total {
            if cancel_flag.load(Ordering::Relaxed) {
                drop(writer);
                let _ = tokio::fs::remove_file(&file_path).await;
                return Err("cancelled".to_string());
            }

            let end = (start + CHUNK - 1).min(total - 1);
            let resp = tokio::time::timeout(
                std::time::Duration::from_secs(RANGE_TIMEOUT_SECS),
                client
                    .get(url)
                    .header(reqwest::header::USER_AGENT, ua)
                    .header("Range", format!("bytes={start}-{end}"))
                    .send(),
            )
            .await
            .map_err(|_| format!("range request timed out after {RANGE_TIMEOUT_SECS}s"))?
            .map_err(|e| format!("Range request failed: {e}"))?;

            let status = resp.status();
            if !status.is_success() {
                return Err(format!("HTTP {status} on range {start}-{end}"));
            }
            // Defensive: a CDN edge ignoring the Range header and
            // returning 200 (full body) plus a CONTENT_LENGTH equal
            // to `total` would otherwise let us write the whole file
            // every iteration (quadratic growth, fills disk). Require
            // 206 Partial Content explicitly.
            if status != reqwest::StatusCode::PARTIAL_CONTENT {
                return Err(format!(
                    "expected 206 Partial Content but got {status} on range {start}-{end} — server ignored Range header"
                ));
            }

            let bytes = resp
                .bytes()
                .await
                .map_err(|e| format!("Range read error: {e}"))?;
            let expected_len = end - start + 1;
            // Defensive: a short read (network hiccup mid-Range)
            // would otherwise advance `start = end + 1` past where
            // bytes actually landed, leaving a zero-filled hole in
            // the output file. Reject and let the retry loop above
            // do its job.
            if bytes.len() as u64 != expected_len {
                return Err(format!(
                    "short read on range {start}-{end}: got {} bytes, expected {expected_len}",
                    bytes.len()
                ));
            }

            writer
                .write_all(&bytes)
                .await
                .map_err(|e| format!("Write: {e}"))?;

            bytes_done += bytes.len() as u64;
            start = end + 1;

            let now = Instant::now();
            let push = !first_update_done
                || now.duration_since(last_update_at).as_millis() >= UI_UPDATE_MS
                || start >= total;
            if push {
                let elapsed = session_start.elapsed().as_secs_f64();
                let trailing = bytes_done - last_update_bytes;
                publish_progress(item_id, bytes_done, trailing, elapsed);
                last_update_at = now;
                last_update_bytes = bytes_done;
                first_update_done = true;
            }
        }

        writer.flush().await.map_err(|e| format!("Flush: {e}"))?;
        let trailing = bytes_done.saturating_sub(last_update_bytes);
        publish_progress(
            item_id,
            bytes_done,
            trailing,
            session_start.elapsed().as_secs_f64(),
        );
        return Ok(file_path);
    }

    let mut req = client.get(url);
    if let Some(ua) = user_agent {
        req = req.header(reqwest::header::USER_AGENT, ua);
    }
    let mut response = req
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }

    let total_bytes = response.content_length().unwrap_or(0);
    let ext = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(super::content_type_to_ext)
        .unwrap_or(ext_hint);

    let file_path = if ext == ext_hint {
        file_path_tentative
    } else {
        dir.join(format!("{item_id}.{ext}"))
    };

    {
        let mut q = queue.write();
        if let Some(item) = q.items.iter_mut().find(|i| i.id == item_id) {
            item.bytes_total = total_bytes;
        }
    }

    let file = tokio::fs::File::create(&file_path)
        .await
        .map_err(|e| format!("Create file: {e}"))?;
    let mut writer = tokio::io::BufWriter::with_capacity(256 * 1024, file);

    let mut bytes_done = 0u64;
    let mut last_update_at = Instant::now();
    let mut last_update_bytes = 0u64;
    let mut first_update_done = false;
    const UI_UPDATE_MS: u128 = 50;
    const CHUNK_TIMEOUT_SECS: u64 = 120;

    loop {
        if cancel_flag.load(Ordering::Relaxed) {
            drop(writer);
            let _ = tokio::fs::remove_file(&file_path).await;
            return Err("cancelled".to_string());
        }

        let chunk_result = tokio::time::timeout(
            std::time::Duration::from_secs(CHUNK_TIMEOUT_SECS),
            response.chunk(),
        )
        .await
        .map_err(|_| format!("chunk timed out after {CHUNK_TIMEOUT_SECS}s"))?
        .map_err(|e| format!("Read error: {e}"))?;

        let chunk = match chunk_result {
            Some(c) => c,
            None => break,
        };

        writer
            .write_all(&chunk)
            .await
            .map_err(|e| format!("Write: {e}"))?;
        bytes_done += chunk.len() as u64;

        let now = Instant::now();
        let push = !first_update_done
            || now.duration_since(last_update_at).as_millis() >= UI_UPDATE_MS
            || (total_bytes > 0 && bytes_done == total_bytes);
        if push {
            let elapsed = session_start.elapsed().as_secs_f64();
            let trailing = bytes_done - last_update_bytes;
            publish_progress(item_id, bytes_done, trailing, elapsed);
            last_update_at = now;
            last_update_bytes = bytes_done;
            first_update_done = true;
        }
    }

    writer.flush().await.map_err(|e| format!("Flush: {e}"))?;
    let trailing = bytes_done.saturating_sub(last_update_bytes);
    publish_progress(
        item_id,
        bytes_done,
        trailing,
        session_start.elapsed().as_secs_f64(),
    );
    Ok(file_path)
}
