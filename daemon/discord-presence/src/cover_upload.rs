//! A local file's embedded cover, put on a temporary image host for Discord,
//! which shows only images a public URL serves.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::cover_art::USER_AGENT;

/// Discord draws the large image at 300 px or less.
const MAX_EDGE: u32 = 512;
const LITTERBOX: &str = "https://litterbox.catbox.moe/resources/internals/api.php";
const UGUU: &str = "https://uguu.se/upload";
const UPLOAD_META_KIND: &str = "discord_uploaded_cover";
/// Litterbox keeps an upload for a day and Uguu for three hours, so one is
/// reused for less than the shorter of the two.
const REUSE_FOR_SECS: u64 = 2 * 60 * 60;

/// The cover's public URL, uploaded now or reused from a recent upload; `None`
/// when the file carries no picture or no host took it.
pub async fn uploaded_cover_url(path: &Path) -> Option<String> {
    let key = cache_key(path)?;
    if let Some(url) = recent_upload(&key).await {
        return Some(url);
    }
    let owned: PathBuf = path.to_path_buf();
    let jpeg = tokio::task::spawn_blocking(move || shrunk_cover(&owned))
        .await
        .ok()??;
    let url = upload(jpeg).await?;
    if let Some(handle) = db::cache::get() {
        let _ = handle
            .meta_put(&key, UPLOAD_META_KIND, &format!("{} {url}", now_unix()))
            .await;
    }
    Some(url)
}

/// The file and its modification time, so a retagged cover is uploaded again.
fn cache_key(path: &Path) -> Option<String> {
    let modified = std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some(format!("{}|{modified}", path.display()))
}

async fn recent_upload(key: &str) -> Option<String> {
    let payload = db::cache::get()?
        .meta_get(key, UPLOAD_META_KIND)
        .await
        .ok()??;
    fresh_upload(&payload, now_unix())
}

fn fresh_upload(payload: &str, now: u64) -> Option<String> {
    let (at, url) = payload.split_once(' ')?;
    let at: u64 = at.parse().ok()?;
    (now.saturating_sub(at) < REUSE_FOR_SECS).then(|| url.to_string())
}

fn shrunk_cover(path: &Path) -> Option<Vec<u8>> {
    let bytes = reader::metadata::embedded_cover_bytes(path)?;
    let image = image::load_from_memory(&bytes).ok()?;
    let image = if image.width() > MAX_EDGE || image.height() > MAX_EDGE {
        image.thumbnail(MAX_EDGE, MAX_EDGE)
    } else {
        image
    };
    let mut jpeg = Vec::new();
    image::DynamicImage::ImageRgb8(image.to_rgb8())
        .write_to(&mut Cursor::new(&mut jpeg), image::ImageFormat::Jpeg)
        .ok()?;
    Some(jpeg)
}

async fn upload(jpeg: Vec<u8>) -> Option<String> {
    let client = utils::http::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(20))
        .build()
        .ok()?;
    match litterbox(&client, jpeg.clone()).await {
        Ok(url) => return Some(url),
        Err(error) => tracing::debug!(%error, "Litterbox did not take the cover"),
    }
    match uguu(&client, jpeg).await {
        Ok(url) => Some(url),
        Err(error) => {
            tracing::warn!(%error, "no image host took the cover for Discord");
            None
        }
    }
}

fn cover_part(jpeg: Vec<u8>) -> Result<reqwest::multipart::Part, String> {
    reqwest::multipart::Part::bytes(jpeg)
        .file_name("cover.jpg")
        .mime_str("image/jpeg")
        .map_err(|error| error.to_string())
}

async fn litterbox(client: &reqwest::Client, jpeg: Vec<u8>) -> Result<String, String> {
    let form = reqwest::multipart::Form::new()
        .text("reqtype", "fileupload")
        .text("time", "24h")
        .part("fileToUpload", cover_part(jpeg)?);
    let body = client
        .post(LITTERBOX)
        .multipart(form)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| error.without_url().to_string())?
        .text()
        .await
        .map_err(|error| error.to_string())?;
    public_url(body.trim())
}

#[derive(Deserialize)]
struct UguuResponse {
    #[serde(default)]
    files: Vec<UguuFile>,
}

#[derive(Deserialize)]
struct UguuFile {
    url: String,
}

async fn uguu(client: &reqwest::Client, jpeg: Vec<u8>) -> Result<String, String> {
    let form = reqwest::multipart::Form::new().part("files[]", cover_part(jpeg)?);
    let response: UguuResponse = client
        .post(UGUU)
        .multipart(form)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| error.without_url().to_string())?
        .json()
        .await
        .map_err(|error| error.to_string())?;
    let url = response
        .files
        .into_iter()
        .next()
        .map(|file| file.url)
        .ok_or("Uguu answered with no file")?;
    public_url(&url)
}

/// A host that fails can answer 200 with an error message as the body.
fn public_url(answer: &str) -> Result<String, String> {
    if answer.starts_with("https://") && !answer.contains(char::is_whitespace) {
        Ok(answer.to_string())
    } else {
        Err(format!(
            "not an image URL: {}",
            answer.chars().take(80).collect::<String>()
        ))
    }
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_upload_is_reused_only_while_both_hosts_still_keep_it() {
        let payload = "1000 https://litter.catbox.moe/abc.jpg";
        assert_eq!(
            fresh_upload(payload, 1000 + 60).as_deref(),
            Some("https://litter.catbox.moe/abc.jpg")
        );
        assert_eq!(fresh_upload(payload, 1000 + REUSE_FOR_SECS), None);
        assert_eq!(fresh_upload("garbage", 1000), None);
    }

    #[test]
    fn only_a_bare_https_url_counts_as_an_upload() {
        assert!(public_url("https://litter.catbox.moe/abc.jpg").is_ok());
        assert!(public_url("Error: file too big").is_err());
        assert!(public_url("http://plain.example/a.jpg").is_err());
    }
}
