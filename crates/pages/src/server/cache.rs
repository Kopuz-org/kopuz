//! Offline-cache path and publication policy.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

pub(super) fn offline_cache_dir() -> PathBuf {
    let base = directories::ProjectDirs::from("com", "temidaradev", "kopuz")
        .map(|dirs| dirs.cache_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("./cache"));
    base.join("offline_tracks")
}

fn safe_extension(extension: &str) -> Result<&str, String> {
    let extension = extension.trim_start_matches('.');
    if extension.is_empty()
        || extension.len() > 8
        || !extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        return Err(format!("invalid download extension: {extension:?}"));
    }
    Ok(extension)
}

/// Derive a cache-local filename without trusting the remote item identifier.
pub(super) fn cache_file_path(item_id: &str, extension: &str) -> Result<PathBuf, String> {
    let extension = safe_extension(extension)?;
    let digest = Sha256::digest(item_id.as_bytes());
    let mut filename = String::with_capacity(digest.len() * 2 + extension.len() + 1);
    for byte in digest {
        let _ = write!(filename, "{byte:02x}");
    }
    filename.push('.');
    filename.push_str(extension);
    Ok(offline_cache_dir().join(filename))
}

/// Delete only files whose parent resolves to the application offline cache.
pub(super) fn remove_cache_file(path: &Path) -> std::io::Result<bool> {
    let cache = offline_cache_dir();
    let cache = std::fs::canonicalize(&cache).unwrap_or(cache);
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

#[tracing::instrument(name = "download.to_cache", skip(url), fields(item_id = %item_id))]
pub async fn download_track_to_cache(
    item_id: &str,
    url: &str,
    extension_hint: &str,
) -> Result<PathBuf, String> {
    download_to_dir(item_id, url, extension_hint).await
}

async fn download_to_dir(
    item_id: &str,
    url: &str,
    extension_hint: &str,
) -> Result<PathBuf, String> {
    let mut response = reqwest::get(url)
        .await
        .map_err(|error| format!("download request failed: {}", error.without_url()))?;
    response
        .error_for_status_ref()
        .map_err(|error| format!("download request failed: {}", error.without_url()))?;

    let extension = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(super::content_type_to_ext)
        .unwrap_or(extension_hint);
    let final_path = cache_file_path(item_id, extension)?;
    let directory = final_path
        .parent()
        .ok_or_else(|| "offline cache path has no parent".to_string())?;
    tokio::fs::create_dir_all(directory)
        .await
        .map_err(|error| format!("create offline cache: {error}"))?;

    let partial_path = final_path.with_extension(format!(
        "{}.part-{}",
        safe_extension(extension)?,
        uuid::Uuid::new_v4()
    ));
    let result = async {
        let file = tokio::fs::File::create(&partial_path)
            .await
            .map_err(|error| format!("create download file: {error}"))?;
        let mut writer = tokio::io::BufWriter::with_capacity(256 * 1024, file);
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| format!("read download response: {}", error.without_url()))?
        {
            writer
                .write_all(&chunk)
                .await
                .map_err(|error| format!("write download file: {error}"))?;
        }
        writer
            .flush()
            .await
            .map_err(|error| format!("flush download file: {error}"))
    }
    .await;

    if let Err(error) = result {
        let _ = tokio::fs::remove_file(&partial_path).await;
        return Err(error);
    }
    if let Err(error) = tokio::fs::rename(&partial_path, &final_path).await {
        let _ = tokio::fs::remove_file(&partial_path).await;
        return Err(format!("publish download file: {error}"));
    }
    Ok(final_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn remote_item_id_cannot_escape_cache_directory() {
        let path = cache_file_path("../../outside", "mp3").expect("valid extension");
        assert_eq!(path.parent(), Some(offline_cache_dir().as_path()));
        assert_eq!(path.extension().and_then(|ext| ext.to_str()), Some("mp3"));
        assert!(!path.to_string_lossy().contains("outside"));
    }

    #[test]
    fn rejects_path_like_extensions() {
        assert!(cache_file_path("track", "../../txt").is_err());
        assert!(cache_file_path("track", "").is_err());
    }

    #[test]
    fn refuses_to_delete_files_outside_the_offline_cache() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("keep.mp3");
        std::fs::write(&path, b"keep").expect("write outside file");

        assert!(!remove_cache_file(&path).expect("safe rejection"));
        assert!(path.exists());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn http_error_is_not_published_as_an_offline_track() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let address = listener.local_addr().expect("test server address");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut request = [0; 1024];
            let _ = stream.read(&mut request);
            stream
                .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 4\r\n\r\nnope")
                .expect("write response");
        });

        let error = download_to_dir("track", &format!("http://{address}"), "mp3")
            .await
            .expect_err("404 must fail");
        server.join().expect("test server");

        assert!(error.contains("404"), "unexpected error: {error}");
    }
}
