//! Entity-addressed artwork: the wire replacement for the wry `artwork://`
//! protocol. Clients ask for a track, album, or artist; the daemon resolves
//! the stored cover ref itself, thumbnails local files (same 400 px / 1920 px
//! policy as the app's protocol handler), and proxies remote covers so
//! credentialed Jellyfin/Subsonic URLs never reach a client.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use api::ApiError;
use sha2::{Digest, Sha256};

use crate::session::SessionHandle;

const THUMB_MAX: u32 = 400;
const HQ_MAX: u32 = 1920;
const HQ_REENCODE_THRESHOLD: usize = 2 * 1024 * 1024;

pub struct ArtworkService {
    db: db::Db,
    session: SessionHandle,
    cache_dir: PathBuf,
}

#[derive(Debug)]
pub struct ArtworkPayload {
    pub bytes: Vec<u8>,
    pub content_type: &'static str,
    pub etag: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtworkEntity<'a> {
    Track(&'a str),
    Album(&'a str),
    Artist(&'a str),
}

/// Reverse of `utils::format_artwork_url`: the path a resolved local cover
/// URL points at. Remote URLs return `None`.
fn local_artwork_path(url: &str) -> Option<String> {
    let query = url
        .strip_prefix("artwork://local")
        .or_else(|| url.strip_prefix("http://artwork.dioxus.localhost/local"))?;
    let query = query.strip_prefix('?').unwrap_or(query);
    let raw = query
        .split('&')
        .find_map(|pair| pair.strip_prefix("p="))
        .unwrap_or(query);
    Some(
        percent_encoding::percent_decode_str(raw)
            .decode_utf8_lossy()
            .to_string(),
    )
}

fn sniff_content_type(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG") {
        "image/png"
    } else if bytes.starts_with(b"GIF8") {
        "image/gif"
    } else if bytes.len() > 11 && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else {
        "image/jpeg"
    }
}

fn hash_name(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    let mut name = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(name, "{byte:02x}");
    }
    name
}

fn shrink_jpeg(raw: &[u8], max_dimension: u32, quality: u8) -> Option<Vec<u8>> {
    use image::codecs::jpeg::JpegEncoder;
    let img = image::load_from_memory(raw).ok()?;
    let img = if img.width() > max_dimension || img.height() > max_dimension {
        img.thumbnail(max_dimension, max_dimension)
    } else {
        img
    };
    let mut out = Vec::new();
    img.write_with_encoder(JpegEncoder::new_with_quality(&mut out, quality))
        .ok()?;
    Some(out)
}

impl ArtworkService {
    pub fn new(db: db::Db, session: SessionHandle, cache_dir: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            db,
            session,
            cache_dir,
        })
    }

    pub async fn fetch(
        &self,
        entity: ArtworkEntity<'_>,
        hq: bool,
    ) -> Result<ArtworkPayload, ApiError> {
        let config = self.session.config_watch().borrow().clone();
        let db_error = |error: db::DbError| ApiError::internal(format!("database error: {error}"));
        let width = if hq { HQ_MAX } else { THUMB_MAX };

        let resolved: Option<String> = match entity {
            ArtworkEntity::Track(key) => {
                let track = self
                    .db
                    .tracks_by_keys(&config.active_source, &[key.to_string()])
                    .await
                    .map_err(db_error)?
                    .into_iter()
                    .next()
                    .ok_or_else(|| ApiError::not_found("unknown track key"))?;
                server::cover::track(&config, &track, width).map(|url| url.as_ref().to_string())
            }
            ArtworkEntity::Album(id) => {
                let album = self
                    .db
                    .album(&config.active_source, id)
                    .await
                    .map_err(db_error)?
                    .ok_or_else(|| ApiError::not_found("unknown album id"))?;
                server::cover::from_path(&config, album.cover_path.as_deref(), width)
                    .map(|url| url.as_ref().to_string())
            }
            ArtworkEntity::Artist(name) => {
                let (overrides, photos) = self.db.artist_images().await.map_err(db_error)?;
                let normalized = name.trim().to_lowercase();
                if let Some(path) = overrides.get(&normalized) {
                    Some(format!(
                        "artwork://local?p={}",
                        percent_encoding::utf8_percent_encode(
                            &path.to_string_lossy(),
                            percent_encoding::NON_ALPHANUMERIC,
                        )
                    ))
                } else {
                    photos.get(&normalized).map(|photo| match photo {
                        reader::ArtistImageRef::Local(path) => format!(
                            "artwork://local?p={}",
                            percent_encoding::utf8_percent_encode(
                                &path.to_string_lossy(),
                                percent_encoding::NON_ALPHANUMERIC,
                            )
                        ),
                        reader::ArtistImageRef::Remote(url) => url.clone(),
                    })
                }
            }
        };

        let Some(resolved) = resolved else {
            return Err(ApiError::not_found("no artwork for this entity"));
        };
        if resolved.starts_with("data:") {
            return Err(ApiError::not_found("no artwork for this entity"));
        }

        if let Some(path) = local_artwork_path(&resolved) {
            self.local_payload(&path, hq).await
        } else if resolved.starts_with("http://") || resolved.starts_with("https://") {
            self.proxied_payload(&resolved).await
        } else {
            self.local_payload(&resolved, hq).await
        }
    }

    /// Thumbnail policy from the app's `artwork_protocol`: 400 px JPEG q75, or
    /// for HQ 1920 px q85 with re-encode only above 2 MiB. Cached on disk.
    async fn local_payload(&self, path: &str, hq: bool) -> Result<ArtworkPayload, ApiError> {
        let cache_path = self.cache_dir.join(format!(
            "{}_{}.jpg",
            if hq { "hq" } else { "thumb" },
            hash_name(path)
        ));
        if let Ok(bytes) = tokio::fs::read(&cache_path).await {
            return Ok(self.payload(bytes, "image/jpeg", &cache_path.to_string_lossy()));
        }
        let raw = tokio::fs::read(path)
            .await
            .map_err(|_| ApiError::not_found("artwork file missing"))?;
        let (bytes, content_type) = if hq && raw.len() <= HQ_REENCODE_THRESHOLD {
            let sniffed = sniff_content_type(&raw);
            (raw, sniffed)
        } else {
            let max = if hq { HQ_MAX } else { THUMB_MAX };
            let quality = if hq { 85 } else { 75 };
            let raw_for_shrink = raw.clone();
            match tokio::task::spawn_blocking(move || shrink_jpeg(&raw_for_shrink, max, quality))
                .await
                .ok()
                .flatten()
            {
                Some(shrunk) => {
                    if let Some(parent) = cache_path.parent() {
                        let _ = tokio::fs::create_dir_all(parent).await;
                    }
                    let _ = tokio::fs::write(&cache_path, &shrunk).await;
                    (shrunk, "image/jpeg")
                }
                None => {
                    let sniffed = sniff_content_type(&raw);
                    (raw, sniffed)
                }
            }
        };
        Ok(self.payload(bytes, content_type, path))
    }

    /// Remote covers are fetched daemon-side (the URL may embed credentials)
    /// and cached on disk keyed by the URL, so a client never sees the origin.
    async fn proxied_payload(&self, url: &str) -> Result<ArtworkPayload, ApiError> {
        let cache_path = self.cache_dir.join(format!("remote_{}", hash_name(url)));
        if let Ok(bytes) = tokio::fs::read(&cache_path).await {
            let content_type = sniff_content_type(&bytes);
            return Ok(self.payload(bytes, content_type, &cache_path.to_string_lossy()));
        }
        let response = reqwest::get(url)
            .await
            .and_then(|response| response.error_for_status())
            .map_err(|error| {
                ApiError::internal(format!("artwork fetch failed: {}", error.without_url()))
            })?;
        let bytes = response
            .bytes()
            .await
            .map_err(|error| {
                ApiError::internal(format!("artwork fetch failed: {}", error.without_url()))
            })?
            .to_vec();
        if let Some(parent) = cache_path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        let _ = tokio::fs::write(&cache_path, &bytes).await;
        let content_type = sniff_content_type(&bytes);
        Ok(self.payload(bytes, content_type, url))
    }

    fn payload(&self, bytes: Vec<u8>, content_type: &'static str, source: &str) -> ArtworkPayload {
        let etag = format!("\"{}-{}\"", hash_name(source), bytes.len());
        ArtworkPayload {
            bytes,
            content_type,
            etag,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artwork_urls_parse_back_to_paths() {
        assert_eq!(
            local_artwork_path("artwork://local?p=%2Ftmp%2Fcover.jpg&v=thumb400-hq1920"),
            Some("/tmp/cover.jpg".to_string())
        );
        assert_eq!(local_artwork_path("https://example.com/a.jpg"), None);
    }

    #[test]
    fn content_type_sniffing_recognizes_magic_bytes() {
        assert_eq!(sniff_content_type(b"\x89PNG\r\n\x1a\n"), "image/png");
        assert_eq!(
            sniff_content_type(b"RIFF\x00\x00\x00\x00WEBPVP8 "),
            "image/webp"
        );
        assert_eq!(sniff_content_type(b"\xff\xd8\xff\xe0"), "image/jpeg");
    }

    #[tokio::test]
    async fn local_track_artwork_serves_a_thumbnail() {
        let dir = tempfile::tempdir().expect("tempdir");
        let database = db::init(&dir.path().join("art.db")).await.expect("db");

        let cover_path = dir.path().join("cover.png");
        let image = image::RgbImage::from_pixel(600, 600, image::Rgb([120, 40, 200]));
        image.save(&cover_path).expect("write cover");

        let track = reader::Track {
            id: reader::TrackId::Local(std::path::PathBuf::from("/lib/art.flac")),
            cover: Some(cover_path.to_string_lossy().into_owned()),
            album_id: "a".into(),
            title: "art".into(),
            artist: String::new(),
            album: String::new(),
            duration: 60,
            khz: 44,
            bitrate: 320,
            track_number: None,
            disc_number: None,
            musicbrainz_release_id: None,
            musicbrainz_recording_id: None,
            musicbrainz_track_id: None,
            playlist_item_id: None,
            artists: vec![],
        };
        database
            .upsert_tracks(&config::Source::Local, &[track])
            .await
            .expect("seed");

        let player =
            player::player::Player::try_with_sink(Box::new(player::engine::NullSink::new()))
                .expect("player");
        let session = SessionHandle::spawn_with_player(
            Arc::new(crate::library::LibraryService::new(
                database.clone(),
                config::Source::Local,
                Arc::new(radio::registry::StationRegistry::default()),
                dir.path().join("covers"),
            )),
            player,
            crate::session::PlaybackServices::default(),
        );
        let service = ArtworkService::new(database, session, dir.path().join("art-cache"));

        let payload = service
            .fetch(ArtworkEntity::Track("/lib/art.flac"), false)
            .await
            .expect("artwork");
        assert_eq!(payload.content_type, "image/jpeg");
        assert!(!payload.bytes.is_empty());
        let thumb = image::load_from_memory(&payload.bytes).expect("decodable");
        assert!(thumb.width() <= THUMB_MAX);

        let cached = service
            .fetch(ArtworkEntity::Track("/lib/art.flac"), false)
            .await
            .expect("cached artwork");
        assert!(cached.etag.len() > 4);

        let missing = service
            .fetch(ArtworkEntity::Track("/nope"), false)
            .await
            .expect_err("unknown track");
        assert_eq!(missing.code, api::ErrorCode::NotFound);
    }
}
