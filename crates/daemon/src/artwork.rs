//! Entity-addressed artwork: the wire replacement for the wry `artwork://`
//! protocol. Clients ask for a track, album, or artist; the daemon resolves
//! the stored cover ref itself, thumbnails local files (same 400 px / 1920 px
//! policy as the app's protocol handler), and proxies remote covers so
//! credentialed Jellyfin/Subsonic URLs never reach a client.

use std::fmt::Write as _;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use api::ApiError;
use sha2::{Digest, Sha256};

use crate::session::SessionHandle;

const THUMB_MAX: u32 = 400;
const HQ_MAX: u32 = 1920;
const MAX_ARTWORK_BYTES: u64 = 32 * 1024 * 1024;
const MAX_INPUT_DIMENSION: u32 = 8192;
const MAX_DECODE_ALLOC: u64 = 192 * 1024 * 1024;
const REMOTE_ARTWORK_TIMEOUT: Duration = Duration::from_secs(15);
const _: () = assert!(MAX_INPUT_DIMENSION >= 6000);
const _: () = assert!(MAX_DECODE_ALLOC >= 6000 * 6000 * 4);

pub struct ArtworkService {
    db: db::Db,
    session: SessionHandle,
    cache_dir: PathBuf,
    decode_slot: Arc<tokio::sync::Semaphore>,
    http: reqwest::Client,
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
    Playlist(&'a str),
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

#[cfg(test)]
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
    let mut reader = image::ImageReader::new(Cursor::new(raw))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_INPUT_DIMENSION);
    limits.max_image_height = Some(MAX_INPUT_DIMENSION);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    reader.limits(limits);
    let img = reader.decode().ok()?;
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
            decode_slot: Arc::new(tokio::sync::Semaphore::new(1)),
            http: reqwest::Client::new(),
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
                    .next();
                let track = match track {
                    Some(track) => track,
                    None => self.session.materialize_track(key.to_string()).await?,
                };
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
            ArtworkEntity::Playlist(id) => {
                let store = self
                    .db
                    .load_playlists(&config.active_source)
                    .await
                    .map_err(db_error)?;
                let playlist = store
                    .playlists
                    .iter()
                    .find(|playlist| playlist.id == id)
                    .ok_or_else(|| ApiError::not_found("unknown playlist id"))?;
                if let Some(url) =
                    server::cover::from_path(&config, playlist.cover_path.as_deref(), width)
                {
                    Some(url.to_string())
                } else if let (Some(tag), Some(server)) =
                    (playlist.image_tag.as_deref(), config.server.as_ref())
                {
                    server::cover::resolve(
                        &config,
                        reader::CoverRef::remote_item(server.service, id, Some(tag)),
                        width,
                    )
                    .map(|url| url.to_string())
                } else if let Some(key) = playlist.tracks.first() {
                    let track = self
                        .db
                        .tracks_by_keys(&config.active_source, std::slice::from_ref(key))
                        .await
                        .map_err(db_error)?
                        .into_iter()
                        .next();
                    track.and_then(|track| {
                        server::cover::track(&config, &track, width)
                            .map(|url| url.as_ref().to_string())
                    })
                } else {
                    None
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
            self.proxied_payload(&resolved, hq).await
        } else {
            self.local_payload(&resolved, hq).await
        }
    }

    async fn local_payload(&self, path: &str, hq: bool) -> Result<ArtworkPayload, ApiError> {
        let metadata = tokio::fs::metadata(path)
            .await
            .map_err(|_| ApiError::not_found("artwork file missing"))?;
        if metadata.len() > MAX_ARTWORK_BYTES {
            return Err(ApiError::invalid_input("artwork file is too large"));
        }
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let cache_path = self.cache_dir.join(format!(
            "{}_{}.jpg",
            if hq { "hq" } else { "thumb" },
            hash_name(&format!("{path}:{}:{modified}", metadata.len()))
        ));
        if let Some(bytes) = read_bounded(&cache_path).await {
            return Ok(self.payload(bytes, "image/jpeg", path));
        }
        let raw = read_bounded(Path::new(path))
            .await
            .ok_or_else(|| ApiError::invalid_input("artwork file is too large"))?;
        let max = if hq { HQ_MAX } else { THUMB_MAX };
        let quality = if hq { 85 } else { 75 };
        let _decode_slot = self
            .decode_slot
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| ApiError::internal("artwork decoder stopped"))?;
        let bytes = tokio::task::spawn_blocking(move || shrink_jpeg(&raw, max, quality))
            .await
            .ok()
            .flatten()
            .ok_or_else(|| ApiError::invalid_input("artwork image is invalid or oversized"))?;
        if let Some(parent) = cache_path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        let _ = tokio::fs::write(&cache_path, &bytes).await;
        Ok(self.payload(bytes, "image/jpeg", path))
    }

    /// Remote covers are fetched daemon-side (the URL may embed credentials)
    /// and cached on disk keyed by the URL, so a client never sees the origin.
    async fn proxied_payload(&self, url: &str, hq: bool) -> Result<ArtworkPayload, ApiError> {
        let cache_path = self.cache_dir.join(format!(
            "remote_{}_{}.jpg",
            if hq { "hq" } else { "thumb" },
            hash_name(url)
        ));
        if let Some(bytes) = read_bounded(&cache_path).await {
            return Ok(self.payload(bytes, "image/jpeg", url));
        }
        let mut response = self
            .http
            .get(url)
            .timeout(REMOTE_ARTWORK_TIMEOUT)
            .send()
            .await
            .map_err(|error| {
                ApiError::internal(format!("artwork fetch failed: {}", error.without_url()))
            })?
            .error_for_status()
            .map_err(|error| {
                ApiError::internal(format!("artwork fetch failed: {}", error.without_url()))
            })?;
        if response
            .content_length()
            .is_some_and(|length| length > MAX_ARTWORK_BYTES)
        {
            return Err(ApiError::internal("remote artwork is too large"));
        }
        let mut raw = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|error| {
            ApiError::internal(format!("artwork fetch failed: {}", error.without_url()))
        })? {
            if raw.len().saturating_add(chunk.len()) > MAX_ARTWORK_BYTES as usize {
                return Err(ApiError::internal("remote artwork is too large"));
            }
            raw.extend_from_slice(&chunk);
        }
        let max = if hq { HQ_MAX } else { THUMB_MAX };
        let quality = if hq { 85 } else { 75 };
        let _decode_slot = self
            .decode_slot
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| ApiError::internal("artwork decoder stopped"))?;
        let bytes = tokio::task::spawn_blocking(move || shrink_jpeg(&raw, max, quality))
            .await
            .ok()
            .flatten()
            .ok_or_else(|| ApiError::invalid_input("remote artwork is invalid or oversized"))?;
        if let Some(parent) = cache_path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        let _ = tokio::fs::write(&cache_path, &bytes).await;
        Ok(self.payload(bytes, "image/jpeg", url))
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

async fn read_bounded(path: &Path) -> Option<Vec<u8>> {
    use tokio::io::AsyncReadExt;

    let file = tokio::fs::File::open(path).await.ok()?;
    let metadata = file.metadata().await.ok()?;
    if metadata.len() > MAX_ARTWORK_BYTES {
        return None;
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_ARTWORK_BYTES + 1)
        .read_to_end(&mut bytes)
        .await
        .ok()?;
    (bytes.len() <= MAX_ARTWORK_BYTES as usize).then_some(bytes)
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
        assert_eq!(cached.etag, payload.etag);

        let missing = service
            .fetch(ArtworkEntity::Track("/nope"), false)
            .await
            .expect_err("unknown track");
        assert_eq!(missing.code, api::ErrorCode::NotFound);
    }
}
