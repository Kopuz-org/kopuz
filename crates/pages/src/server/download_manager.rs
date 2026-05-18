use config::AppConfig;
use dioxus::prelude::*;
use std::time::Instant;

pub use ::server::{DownloadItem, DownloadQueue, DownloadStatus};

#[cfg(not(target_arch = "wasm32"))]
pub fn queue_downloads(
    requests: Vec<(String, String, String)>,
    mut config: Signal<AppConfig>,
    mut queue: Signal<DownloadQueue>,
) {
    let mut added = false;
    {
        let mut q = queue.write();
        q.cancel_requested = false;
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
        q.is_running = true;
    }

    let session_start = Instant::now();
    spawn(async move {
        loop {
            if queue.read().cancel_requested {
                let mut q = queue.write();
                q.is_running = false;
                q.cancel_requested = false;
                return;
            }

            let next_id = {
                let q = queue.read();
                q.items
                    .iter()
                    .find(|i| matches!(i.status, DownloadStatus::Queued))
                    .map(|i| i.id.clone())
            };

            let id = match next_id {
                Some(id) => id,
                None => {
                    let mut q = queue.write();
                    let still_empty = !q
                        .items
                        .iter()
                        .any(|i| matches!(i.status, DownloadStatus::Queued));
                    if still_empty {
                        q.is_running = false;
                        return;
                    }
                    continue;
                }
            };

            if config.read().offline_tracks.contains_key(&id) {
                if let Some(item) = queue.write().items.iter_mut().find(|i| i.id == id) {
                    item.status = DownloadStatus::Done;
                }
                continue;
            }

            let url_result = {
                let conf = config.read();
                super::build_download_url(&id, &conf)
            };

            let (url, ext_hint) = match url_result {
                Some(v) => v,
                None => {
                    if let Some(item) = queue.write().items.iter_mut().find(|i| i.id == id) {
                        item.status = DownloadStatus::Failed;
                    }
                    continue;
                }
            };

            if let Some(item) = queue.write().items.iter_mut().find(|i| i.id == id) {
                item.status = DownloadStatus::Downloading;
            }

            match download_with_progress(&id, &url, ext_hint, &mut queue, &session_start).await {
                Ok(path) => {
                    config
                        .write()
                        .offline_tracks
                        .insert(id.clone(), path.to_string_lossy().into_owned());
                    if let Some(item) = queue.write().items.iter_mut().find(|i| i.id == id) {
                        item.status = DownloadStatus::Done;
                    }
                }
                Err(e) => {
                    eprintln!("Download failed for {id}: {e}");
                    if let Some(item) = queue.write().items.iter_mut().find(|i| i.id == id) {
                        item.status = DownloadStatus::Failed;
                    }
                }
            }
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub fn delete_downloads(
    ids: Vec<String>,
    mut config: Signal<AppConfig>,
    mut queue: Signal<DownloadQueue>,
) {
    let mut conf = config.write();
    let mut q = queue.write();

    for id in ids {
        if let Some(path_str) = conf.offline_tracks.remove(&id) {
            let path = std::path::Path::new(&path_str);
            if path.exists() {
                let _ = std::fs::remove_file(path);
            }
        }
        q.items.retain(|i| i.id != id);
    }
}

#[cfg(not(target_arch = "wasm32"))]
async fn download_with_progress(
    item_id: &str,
    url: &str,
    ext_hint: &'static str,
    queue: &mut Signal<DownloadQueue>,
    session_start: &Instant,
) -> Result<std::path::PathBuf, String> {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("Client build error: {e}"))?;

    let mut response = client
        .get(url)
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

    {
        let mut q = queue.write();
        if let Some(item) = q.items.iter_mut().find(|i| i.id == item_id) {
            item.bytes_total = total_bytes;
        }
    }

    let mut bytes_vec: Vec<u8> = Vec::with_capacity(total_bytes.max(65536) as usize);
    let mut bytes_done = 0u64;
    let mut last_update_bytes = 0u64;
    const UPDATE_INTERVAL: u64 = 65536;
    const CHUNK_TIMEOUT_SECS: u64 = 120;

    loop {
        if queue.read().cancel_requested {
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

        bytes_done += chunk.len() as u64;
        bytes_vec.extend_from_slice(&chunk);

        if bytes_done - last_update_bytes >= UPDATE_INTERVAL || bytes_done == total_bytes {
            let elapsed = session_start.elapsed().as_secs_f64();
            let chunk_bytes = bytes_done - last_update_bytes;
            let mut q = queue.write();
            if let Some(item) = q.items.iter_mut().find(|i| i.id == item_id) {
                item.bytes_done = bytes_done;
            }
            q.bytes_done_session += chunk_bytes;
            q.session_elapsed_secs = elapsed;
            last_update_bytes = bytes_done;
        }
    }

    let dir = super::offline_cache_dir();
    let file_path = dir.join(format!("{item_id}.{ext}"));
    tokio::fs::write(&file_path, &bytes_vec)
        .await
        .map_err(|e| format!("Write failed: {e}"))?;

    Ok(file_path)
}
