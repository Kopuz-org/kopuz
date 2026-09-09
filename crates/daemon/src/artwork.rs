//! Entity-addressed artwork: clients name a track, album, artist or playlist
//! and get bytes back.
//!
//! One resolution chain per entity, used twice. The `*_cover` functions say
//! where an entity's picture lives; [`ArtworkService::fetch`] turns that into
//! bytes, and the `*_ref` wrappers turn it into the [`ArtworkRef`] a library
//! row advertises. Because both walk the same chain, a row claims a cover
//! exactly when asking for one would produce it -- which is what lets a
//! client draw a placeholder without making a request.
//!
//! The daemon resolves covers because it holds the credentials that sign a
//! Jellyfin or Subsonic image URL, and it proxies remote ones so those URLs
//! never reach a client.

use std::fmt::Write as _;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use api::{ApiError, ArtworkRef, ArtworkTarget};
use reader::CoverRef;
use server::cover::Located;
use sha2::{Digest, Sha256};

use crate::session::SessionHandle;

use utils::artwork_image::{
    HQ_MAX, HQ_QUALITY, HQ_REENCODE_THRESHOLD, THUMB_MAX, THUMB_QUALITY, shrink_jpeg,
};
const MAX_REMOTE_ARTWORK_BYTES: usize = 32 * 1024 * 1024;
const REMOTE_ARTWORK_TIMEOUT: Duration = Duration::from_secs(15);

/// The version an [`ArtworkRef`] carries: a hash of the resolved cover
/// reference, so it changes when and only when the picture would -- a photo
/// search lands, a scan indexes a cover, a server rotates its tag.
///
/// It hashes a stored key (a path, an image tag, an item id), never a signed
/// URL, so a client may cache under it without holding anything secret.
fn version_of(cover: &CoverRef) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    cover.hash(&mut hasher);
    hasher.finish()
}

/// The one place "this entity has no picture" is decided.
fn ref_for(target: ArtworkTarget, cover: CoverRef) -> Option<ArtworkRef> {
    match cover {
        CoverRef::None => None,
        cover => Some(ArtworkRef::new(target, version_of(&cover))),
    }
}

/// An artist's picture: a custom override, then the source's own photo, then
/// -- for a library source only -- one of their album covers.
///
/// That last resort is what keeps a local grid from being a wall of
/// placeholders. A remote catalog never uses it: a liked track's album cover
/// is not a picture of the artist.
pub fn artist_cover(
    name: &str,
    images: &db::ArtistImages,
    album_cover: Option<&Path>,
    library_view: bool,
) -> CoverRef {
    let normalized = name.trim().to_lowercase();
    let (overrides, photos) = images;
    if let Some(path) = overrides.get(&normalized) {
        return CoverRef::Local(path.clone());
    }
    if let Some(photo) = photos.get(&normalized) {
        return match photo {
            reader::ArtistImageRef::Local(path) => CoverRef::Local(path.clone()),
            reader::ArtistImageRef::Remote(url) => CoverRef::EmbeddedUrl(url.clone()),
        };
    }
    match album_cover.filter(|_| library_view) {
        Some(path) => CoverRef::parse(&path.to_string_lossy()),
        None => CoverRef::None,
    }
}

/// A playlist's cover: an explicit one, then the server's image tag, then the
/// first track's art. Only the daemon can sign the middle one.
pub fn playlist_cover(
    playlist: &reader::models::Playlist,
    config: &config::AppConfig,
    first_track: Option<&reader::Track>,
) -> CoverRef {
    if let Some(path) = playlist.cover_path.as_ref() {
        let explicit = CoverRef::parse(&path.to_string_lossy());
        if explicit != CoverRef::None {
            return explicit;
        }
    }
    if let (Some(tag), Some(server)) = (playlist.image_tag.as_ref(), config.server.as_ref()) {
        let tagged = CoverRef::remote_item(server.service, &playlist.id, Some(tag));
        if tagged != CoverRef::None {
            return tagged;
        }
    }
    first_track.map_or(CoverRef::None, CoverRef::for_track)
}

pub fn album_cover(album: &reader::Album) -> CoverRef {
    match album.cover_path.as_ref() {
        Some(path) => CoverRef::parse(&path.to_string_lossy()),
        None => CoverRef::None,
    }
}

pub fn track_ref(track: &reader::Track) -> Option<ArtworkRef> {
    ref_for(
        ArtworkTarget::Track(track.id.key().into_owned()),
        CoverRef::for_track(track),
    )
}

pub fn album_ref(album: &reader::Album) -> Option<ArtworkRef> {
    ref_for(ArtworkTarget::Album(album.id.clone()), album_cover(album))
}

pub fn artist_ref(
    name: &str,
    images: &db::ArtistImages,
    album_cover: Option<&Path>,
    library_view: bool,
) -> Option<ArtworkRef> {
    ref_for(
        ArtworkTarget::Artist(name.to_string()),
        artist_cover(name, images, album_cover, library_view),
    )
}

pub fn playlist_ref(
    playlist: &reader::models::Playlist,
    config: &config::AppConfig,
    first_track: Option<&reader::Track>,
) -> Option<ArtworkRef> {
    ref_for(
        ArtworkTarget::Playlist(playlist.id.clone()),
        playlist_cover(playlist, config, first_track),
    )
}

pub struct ArtworkService {
    db: db::Db,
    session: SessionHandle,
    cache_dir: PathBuf,
    http: reqwest::Client,
}

#[derive(Debug)]
pub struct ArtworkPayload {
    pub bytes: Vec<u8>,
    pub content_type: &'static str,
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

impl ArtworkService {
    pub fn new(db: db::Db, session: SessionHandle, cache_dir: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            db,
            session,
            cache_dir,
            http: reqwest::Client::new(),
        })
    }

    pub async fn fetch(
        &self,
        target: &ArtworkTarget,
        hq: bool,
    ) -> Result<ArtworkPayload, ApiError> {
        let config = self.session.config_watch().borrow().clone();
        let width = if hq { HQ_MAX } else { THUMB_MAX };
        let cover = self.cover_for(target, &config).await?;
        let missing = || ApiError::not_found("no artwork for this entity");
        match server::cover::locate(&config, cover, width).ok_or_else(missing)? {
            Located::File(path) => self.local_payload(&path.to_string_lossy(), hq).await,
            Located::Url(url) => self.proxied_payload(&url).await,
        }
    }

    /// Load the entity and walk its cover chain -- the same chain the `*_ref`
    /// functions walk when a row advertises artwork.
    async fn cover_for(
        &self,
        target: &ArtworkTarget,
        config: &config::AppConfig,
    ) -> Result<CoverRef, ApiError> {
        let db_error = |error: db::DbError| ApiError::internal(format!("database error: {error}"));
        match target {
            ArtworkTarget::Track(key) => Ok(CoverRef::for_track(&self.track(key, config).await?)),
            ArtworkTarget::Album(id) => {
                let album = self
                    .db
                    .album(&config.active_source, id)
                    .await
                    .map_err(db_error)?
                    .ok_or_else(|| ApiError::not_found("unknown album id"))?;
                Ok(album_cover(&album))
            }
            ArtworkTarget::Artist(name) => {
                let images = self.db.artist_images().await.map_err(db_error)?;
                let source = server::source::active(self.db.clone(), config);
                let library_view =
                    source.capabilities().artist_view == server::source::ArtistView::Library;
                let album = if library_view {
                    self.artist_album_cover(name, config).await?
                } else {
                    None
                };
                Ok(artist_cover(name, &images, album.as_deref(), library_view))
            }
            ArtworkTarget::Playlist(id) => {
                let store = self
                    .db
                    .load_playlists(&config.active_source)
                    .await
                    .map_err(db_error)?;
                let playlist = store
                    .playlists
                    .iter()
                    .find(|playlist| &playlist.id == id)
                    .ok_or_else(|| ApiError::not_found("unknown playlist id"))?;
                let first = match playlist.tracks.first() {
                    Some(key) => self.track(key, config).await.ok(),
                    None => None,
                };
                Ok(playlist_cover(playlist, config, first.as_ref()))
            }
            ArtworkTarget::Catalog(_) | ArtworkTarget::Station(_) => Err(ApiError::unsupported(
                "this daemon serves no catalog or station artwork",
            )),
        }
    }

    /// A track the library holds, or one the session is playing that came
    /// from a live listing the database has never seen.
    async fn track(
        &self,
        key: &str,
        config: &config::AppConfig,
    ) -> Result<reader::Track, ApiError> {
        let found = self
            .db
            .tracks_by_keys(&config.active_source, &[key.to_string()])
            .await
            .map_err(|error| ApiError::internal(format!("database error: {error}")))?
            .into_iter()
            .next();
        match found {
            Some(track) => Ok(track),
            None => self
                .session
                .queued_track(key)
                .await
                .ok_or_else(|| ApiError::not_found("unknown track key")),
        }
    }

    async fn artist_album_cover(
        &self,
        name: &str,
        config: &config::AppConfig,
    ) -> Result<Option<PathBuf>, ApiError> {
        let normalized = name.trim().to_lowercase();
        Ok(self
            .db
            .albums(&config.active_source)
            .await
            .map_err(|error| ApiError::internal(format!("database error: {error}")))?
            .into_iter()
            .find(|album| {
                album.cover_path.is_some() && album.artist.trim().to_lowercase() == normalized
            })
            .and_then(|album| album.cover_path))
    }

    /// Resized by the shared policy in `utils::artwork_image`, then cached on
    /// disk; an HQ original under the re-encode threshold is served as-is.
    async fn local_payload(&self, path: &str, hq: bool) -> Result<ArtworkPayload, ApiError> {
        let cache_path = self.cache_dir.join(format!(
            "{}_{}.jpg",
            if hq { "hq" } else { "thumb" },
            hash_name(path)
        ));
        if let Ok(bytes) = tokio::fs::read(&cache_path).await {
            return Ok(self.payload(bytes, "image/jpeg"));
        }
        let raw = tokio::fs::read(path)
            .await
            .map_err(|_| ApiError::not_found("artwork file missing"))?;
        let (bytes, content_type) = if hq && raw.len() <= HQ_REENCODE_THRESHOLD {
            let sniffed = sniff_content_type(&raw);
            (raw, sniffed)
        } else {
            let max = if hq { HQ_MAX } else { THUMB_MAX };
            let quality = if hq { HQ_QUALITY } else { THUMB_QUALITY };
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
        Ok(self.payload(bytes, content_type))
    }

    /// Remote covers are fetched daemon-side (the URL may embed credentials)
    /// and cached on disk keyed by the URL, so a client never sees the origin.
    async fn proxied_payload(&self, url: &str) -> Result<ArtworkPayload, ApiError> {
        let cache_path = self.cache_dir.join(format!("remote_{}", hash_name(url)));
        if let Ok(bytes) = tokio::fs::read(&cache_path).await {
            let content_type = sniff_content_type(&bytes);
            return Ok(self.payload(bytes, content_type));
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
            .is_some_and(|length| length > MAX_REMOTE_ARTWORK_BYTES as u64)
        {
            return Err(ApiError::internal("artwork exceeds the maximum size"));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|error| {
            ApiError::internal(format!("artwork fetch failed: {}", error.without_url()))
        })? {
            if bytes.len().saturating_add(chunk.len()) > MAX_REMOTE_ARTWORK_BYTES {
                return Err(ApiError::internal("artwork exceeds the maximum size"));
            }
            bytes.extend_from_slice(&chunk);
        }
        if let Some(parent) = cache_path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        let _ = tokio::fs::write(&cache_path, &bytes).await;
        let content_type = sniff_content_type(&bytes);
        Ok(self.payload(bytes, content_type))
    }

    fn payload(&self, bytes: Vec<u8>, content_type: &'static str) -> ArtworkPayload {
        ArtworkPayload {
            bytes,
            content_type,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track_with_cover(cover: Option<&str>) -> reader::Track {
        reader::Track {
            id: reader::TrackId::Local(std::path::PathBuf::from("/lib/art.flac")),
            cover: cover.map(str::to_string),
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
        }
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

    /// A row without a cover advertises nothing, so a client draws the
    /// placeholder instead of asking for a picture that does not exist.
    #[test]
    fn a_row_without_a_cover_advertises_no_artwork() {
        assert!(track_ref(&track_with_cover(None)).is_none());
        assert!(track_ref(&track_with_cover(Some(reader::CoverRef::NO_COVER))).is_none());
        assert!(
            album_ref(&reader::Album {
                id: "al".into(),
                title: String::new(),
                artist: String::new(),
                genre: String::new(),
                year: 0,
                cover_path: None,
                manual_cover: false,
            })
            .is_none()
        );
    }

    #[test]
    fn the_version_follows_the_cover_and_nothing_else() {
        let first = track_ref(&track_with_cover(Some("/music/a.jpg"))).expect("artwork");
        let same = track_ref(&track_with_cover(Some("/music/a.jpg"))).expect("artwork");
        let other = track_ref(&track_with_cover(Some("/music/b.jpg"))).expect("artwork");
        assert_eq!(first.version, same.version, "same cover, same version");
        assert_ne!(first.version, other.version, "new cover, new version");
        assert_eq!(first.target, other.target, "the target is the identity");
    }

    /// The artist chain the grid depends on: an override wins, then the
    /// source's photo, then an album cover -- but only for a library source.
    #[test]
    fn artist_art_falls_back_through_override_photo_then_album() {
        let album = std::path::Path::new("/music/band/cover.jpg");
        let mut overrides = std::collections::HashMap::new();
        let mut photos = std::collections::HashMap::new();
        photos.insert(
            "band".to_string(),
            reader::ArtistImageRef::Remote("https://p/band.jpg".into()),
        );
        let images: db::ArtistImages = (overrides.clone(), photos.clone());
        assert_eq!(
            artist_cover("Band", &images, Some(album), true),
            CoverRef::EmbeddedUrl("https://p/band.jpg".into())
        );

        overrides.insert("band".to_string(), PathBuf::from("/pics/band.png"));
        let images: db::ArtistImages = (overrides, photos);
        assert_eq!(
            artist_cover("Band", &images, Some(album), true),
            CoverRef::Local(PathBuf::from("/pics/band.png"))
        );

        let empty: db::ArtistImages = Default::default();
        assert_eq!(
            artist_cover("Band", &empty, Some(album), true),
            CoverRef::Local(album.to_path_buf()),
            "a library artist may borrow an album cover"
        );
        assert_eq!(
            artist_cover("Band", &empty, Some(album), false),
            CoverRef::None,
            "a remote catalog never renders an album as the artist"
        );
    }
}
