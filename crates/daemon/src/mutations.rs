//! Changing the library: tags, deletions, artwork.
//!
//! These write files and push to servers, so they run where the credentials
//! and the library roots are. A caller names a track by key and says what it
//! wants changed; it never passes a path, and a path that would escape the
//! configured roots is refused rather than trusted.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use api::{ApiError, ArtworkChange, ArtworkTarget, ArtworkUpload, Table, TrackMetadataPatch};
use sha2::{Digest, Sha256};

use crate::session::SessionHandle;

/// Big enough for a scanned LP sleeve, small enough that a mistake is not a
/// denial of service.
const MAX_ARTWORK_BYTES: usize = 32 * 1024 * 1024;

pub struct MutationService {
    db: db::Db,
    session: SessionHandle,
    /// Where uploaded pictures live, content-addressed.
    uploads: PathBuf,
}

fn db_error(error: db::DbError) -> ApiError {
    ApiError::internal(format!("database error: {error}"))
}

fn source_error(error: server::source::SourceError) -> ApiError {
    use api::ErrorCode;
    use server::source::SourceError;
    match &error {
        SourceError::Unsupported(what) => ApiError::unsupported(*what),
        SourceError::Auth => ApiError::new(ErrorCode::SourceAuthExpired, error.to_string()),
        SourceError::Connectivity => ApiError::new(ErrorCode::SourceUnreachable, error.to_string()),
        SourceError::InvalidInput(message) => ApiError::invalid_input(message.clone()),
        SourceError::Backend(message) => ApiError::internal(message.clone()),
    }
}

/// The roots the active source may delete inside. A server source has none:
/// its files are not ours to unlink.
fn configured_roots(config: &config::AppConfig) -> Vec<&Path> {
    match &config.active_source {
        config::Source::Local => config
            .music_directory
            .iter()
            .map(PathBuf::as_path)
            .collect(),
        config::Source::LocalLibrary(id) => config
            .local_sources
            .iter()
            .find(|saved| &saved.id == id)
            .map(|saved| saved.directories.iter().map(PathBuf::as_path).collect())
            .unwrap_or_default(),
        config::Source::Server(_) => Vec::new(),
    }
}

/// Whether a path is genuinely inside a configured root, canonicalized on
/// both sides so a symlink or `..` cannot walk out of the library.
fn inside_a_root(config: &config::AppConfig, path: &Path) -> bool {
    let Ok(target) = std::fs::canonicalize(path) else {
        return false;
    };
    configured_roots(config).into_iter().any(|root| {
        std::fs::canonicalize(root).is_ok_and(|root| target != root && target.starts_with(&root))
    })
}

impl MutationService {
    pub fn new(db: db::Db, session: SessionHandle, uploads: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            db,
            session,
            uploads,
        })
    }

    fn config(&self) -> config::AppConfig {
        self.session.config_watch().borrow().clone()
    }

    fn source(&self) -> server::source::ActiveSource {
        Arc::from(server::source::active(self.db.clone(), &self.config()))
    }

    async fn track(&self, key: &str) -> Result<reader::Track, ApiError> {
        self.db
            .tracks_by_keys(&self.config().active_source, &[key.to_string()])
            .await
            .map_err(db_error)?
            .into_iter()
            .next()
            .ok_or_else(|| ApiError::not_found("track not found"))
    }

    /// The file behind a track, once it is established that we may write it.
    fn editable_path(
        config: &config::AppConfig,
        track: &reader::Track,
    ) -> Result<PathBuf, ApiError> {
        let path = track
            .id
            .local_path()
            .ok_or_else(|| ApiError::unsupported("only local files have tags to edit"))?;
        if !inside_a_root(config, path) {
            return Err(ApiError::invalid_input(
                "that file is outside the configured library roots",
            ));
        }
        Ok(path.to_path_buf())
    }

    pub async fn update_track_metadata(
        &self,
        patch: TrackMetadataPatch,
    ) -> Result<api::TrackInfo, ApiError> {
        let config = self.config();
        let mut track = self.track(&patch.key).await?;
        let path = Self::editable_path(&config, &track)?;

        let title = patch.title.unwrap_or_else(|| track.title.clone());
        let artist = patch.artist.unwrap_or_else(|| track.artist.clone());
        let album = patch.album.unwrap_or_else(|| track.album.clone());
        let track_number = if patch.clear_track_number {
            None
        } else {
            patch.track_number.or(track.track_number)
        };
        let disc_number = if patch.clear_disc_number {
            None
        } else {
            patch.disc_number.or(track.disc_number)
        };
        let cover = match patch.cover {
            ArtworkChange::Keep => reader::CoverChange::Keep,
            ArtworkChange::Remove => reader::CoverChange::Remove,
            ArtworkChange::Set(bytes) => {
                validate_image(&bytes)?;
                reader::CoverChange::Set(bytes)
            }
        };

        let edits = reader::TrackEdits {
            title: title.clone(),
            artist: artist.clone(),
            album: album.clone(),
            track_number,
            disc_number,
            cover,
        };
        tokio::task::spawn_blocking(move || reader::write_tags(&path, &edits))
            .await
            .map_err(|error| ApiError::internal(format!("tag writer task failed: {error}")))?
            .map_err(ApiError::internal)?;

        track.title = title.trim().to_string();
        track.artist = artist.trim().to_string();
        // Credits are re-derived rather than patched: the single artist
        // string is what the user edited, so it is the authority.
        track.artists = artist
            .split([';', ','])
            .map(str::trim)
            .filter(|artist| !artist.is_empty())
            .map(str::to_string)
            .collect();
        track.album = album.trim().to_string();
        track.album_id = reader::metadata::make_album_id(&track.album, &track.artist);
        track.track_number = track_number;
        track.disc_number = disc_number;
        self.db
            .upsert_tracks(&config.active_source, std::slice::from_ref(&track))
            .await
            .map_err(db_error)?;
        self.session.invalidate(Table::Tracks);
        Ok(crate::wire::track_info(&track, &config))
    }

    pub async fn delete_tracks(&self, keys: &[String], from_disk: bool) -> Result<(), ApiError> {
        let config = self.config();
        if from_disk {
            let tracks = self
                .db
                .tracks_by_keys(&config.active_source, keys)
                .await
                .map_err(db_error)?;
            // Every path is checked before any file is removed, so a refusal
            // does not leave the deletion half-done.
            let mut paths = Vec::with_capacity(tracks.len());
            for track in &tracks {
                paths.push(Self::editable_path(&config, track)?);
            }
            tokio::task::spawn_blocking(move || {
                for path in paths {
                    if let Err(error) = std::fs::remove_file(&path)
                        && error.kind() != std::io::ErrorKind::NotFound
                    {
                        return Err(error);
                    }
                }
                Ok::<(), std::io::Error>(())
            })
            .await
            .map_err(|error| ApiError::internal(format!("delete task failed: {error}")))?
            .map_err(|error| ApiError::internal(format!("file delete failed: {error}")))?;
        }
        self.source()
            .delete_tracks(keys)
            .await
            .map_err(source_error)?;
        self.session.invalidate(Table::Tracks);
        Ok(())
    }

    pub async fn delete_album(&self, id: &str, from_disk: bool) -> Result<(), ApiError> {
        if from_disk {
            let keys: Vec<String> = self
                .db
                .album_tracks(&self.config().active_source, id)
                .await
                .map_err(db_error)?
                .iter()
                .map(|track| track.id.key().into_owned())
                .collect();
            self.delete_tracks(&keys, true).await?;
        }
        self.source().delete_album(id).await.map_err(source_error)?;
        self.session.invalidate(Table::Albums);
        self.session.invalidate(Table::Tracks);
        Ok(())
    }

    pub async fn upload_artwork(&self, upload: ArtworkUpload) -> Result<(), ApiError> {
        validate_image(&upload.bytes)?;
        if let ArtworkTarget::Track(key) = &upload.target {
            return self
                .set_track_cover(key, ArtworkChange::Set(upload.bytes))
                .await;
        }
        let extension = match upload.content_type.as_str() {
            "image/jpeg" => "jpg",
            "image/png" => "png",
            "image/webp" => "webp",
            other => {
                return Err(ApiError::invalid_input(format!(
                    "unsupported artwork content type: {other}"
                )));
            }
        };
        // Content-addressed, so re-uploading the same picture is idempotent
        // and a different one lands at a different path -- which is what makes
        // the artwork version, and the caches keyed by it, change.
        let mut name = String::with_capacity(72);
        for byte in Sha256::digest(&upload.bytes) {
            use std::fmt::Write as _;
            let _ = write!(name, "{byte:02x}");
        }
        let path = self.uploads.join(format!("{name}.{extension}"));
        tokio::fs::create_dir_all(&self.uploads)
            .await
            .map_err(|error| ApiError::internal(format!("artwork directory failed: {error}")))?;
        tokio::fs::write(&path, &upload.bytes)
            .await
            .map_err(|error| ApiError::internal(format!("artwork write failed: {error}")))?;
        let stored = path.to_string_lossy().into_owned();

        let previous = self.current_artwork_path(&upload.target).await?;
        let result = match &upload.target {
            ArtworkTarget::Album(id) => self
                .source()
                .update_album_cover(id, Some(&stored), true)
                .await
                .map_err(source_error)
                .map(|_| Table::Albums),
            ArtworkTarget::Artist(name) => self
                .source()
                .set_artist_image(
                    &utils::artist::normalize_artist_key(name),
                    "custom",
                    Some(&stored),
                )
                .await
                .map_err(source_error)
                .map(|_| Table::Tracks),
            ArtworkTarget::Playlist(id) => {
                let playlist = self.playlist(id).await?;
                self.source()
                    .set_playlist_cover(id, &playlist.name, &path, playlist.image_tag.as_deref())
                    .await
                    .map_err(source_error)
                    .map(|_| Table::Playlists)
            }
            ArtworkTarget::Track(_) => unreachable!("handled above"),
            ArtworkTarget::Catalog(_) | ArtworkTarget::Station(_) => Err(ApiError::unsupported(
                "catalog and station artwork is not ours to set",
            )),
        };
        match result {
            Ok(table) => {
                self.session.invalidate(table);
                self.forget_upload(previous, &path).await;
                Ok(())
            }
            Err(error) => {
                let _ = tokio::fs::remove_file(&path).await;
                Err(error)
            }
        }
    }

    pub async fn remove_artwork(&self, target: ArtworkTarget) -> Result<(), ApiError> {
        if let ArtworkTarget::Track(key) = &target {
            return self.set_track_cover(key, ArtworkChange::Remove).await;
        }
        let previous = self.current_artwork_path(&target).await?;
        let table = match &target {
            ArtworkTarget::Album(id) => {
                self.source()
                    .update_album_cover(id, None, false)
                    .await
                    .map_err(source_error)?;
                Table::Albums
            }
            ArtworkTarget::Artist(name) => {
                self.source()
                    .set_artist_image(&utils::artist::normalize_artist_key(name), "custom", None)
                    .await
                    .map_err(source_error)?;
                Table::Tracks
            }
            ArtworkTarget::Playlist(id) => {
                let playlist = self.playlist(id).await?;
                self.db
                    .upsert_playlist_meta(
                        &self.config().active_source,
                        id,
                        &playlist.name,
                        None,
                        playlist.image_tag.as_deref(),
                    )
                    .await
                    .map_err(db_error)?;
                Table::Playlists
            }
            ArtworkTarget::Track(_) => unreachable!("handled above"),
            ArtworkTarget::Catalog(_) | ArtworkTarget::Station(_) => {
                return Err(ApiError::unsupported(
                    "catalog and station artwork is not ours to remove",
                ));
            }
        };
        self.session.invalidate(table);
        self.forget_upload(previous, Path::new("")).await;
        Ok(())
    }

    /// A track's cover lives in its tags, so changing it is a tag write.
    async fn set_track_cover(&self, key: &str, cover: ArtworkChange) -> Result<(), ApiError> {
        self.update_track_metadata(TrackMetadataPatch {
            key: key.to_string(),
            cover,
            ..Default::default()
        })
        .await
        .map(|_| ())
    }

    async fn playlist(&self, id: &str) -> Result<reader::models::Playlist, ApiError> {
        self.db
            .load_playlists(&self.config().active_source)
            .await
            .map_err(db_error)?
            .playlists
            .into_iter()
            .find(|playlist| playlist.id == id)
            .ok_or_else(|| ApiError::not_found("playlist not found"))
    }

    async fn current_artwork_path(
        &self,
        target: &ArtworkTarget,
    ) -> Result<Option<PathBuf>, ApiError> {
        let config = self.config();
        Ok(match target {
            ArtworkTarget::Album(id) => self
                .db
                .album(&config.active_source, id)
                .await
                .map_err(db_error)?
                .and_then(|album| album.cover_path),
            ArtworkTarget::Artist(name) => self
                .db
                .artist_images()
                .await
                .map_err(db_error)?
                .0
                .get(&utils::artist::normalize_artist_key(name))
                .cloned(),
            ArtworkTarget::Playlist(id) => self.playlist(id).await?.cover_path,
            _ => None,
        })
    }

    /// Delete a picture this daemon uploaded once nothing points at it.
    /// Anything outside the upload directory belongs to the library and is
    /// left alone.
    async fn forget_upload(&self, previous: Option<PathBuf>, keep: &Path) {
        if let Some(previous) = previous
            && previous.starts_with(&self.uploads)
            && previous != keep
        {
            let _ = tokio::fs::remove_file(previous).await;
        }
    }
}

/// Decode the image before storing it, with bounds, so a malformed or
/// enormous upload fails here rather than in whatever renders it.
fn validate_image(bytes: &[u8]) -> Result<(), ApiError> {
    if bytes.is_empty() || bytes.len() > MAX_ARTWORK_BYTES {
        return Err(ApiError::invalid_input(
            "artwork must be between 1 byte and 32 MiB",
        ));
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| ApiError::invalid_input(format!("unreadable image: {error}")))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    reader
        .decode()
        .map_err(|error| ApiError::invalid_input(format!("undecodable image: {error}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with_root(root: &Path) -> config::AppConfig {
        config::AppConfig {
            active_source: config::Source::Local,
            music_directory: vec![root.to_path_buf()],
            ..Default::default()
        }
    }

    /// The guard that makes `from_disk` safe to expose on a socket: a path
    /// outside the library, or reached by walking out of it, is not ours.
    #[test]
    fn only_files_inside_a_configured_root_are_deletable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let library = dir.path().join("library");
        std::fs::create_dir(&library).expect("create library");
        let inside = library.join("song.flac");
        let outside = dir.path().join("elsewhere.flac");
        std::fs::write(&inside, b"in").expect("write inside");
        std::fs::write(&outside, b"out").expect("write outside");
        let config = config_with_root(&library);

        assert!(inside_a_root(&config, &inside));
        assert!(!inside_a_root(&config, &outside));
        assert!(
            !inside_a_root(&config, &library.join("..").join("elsewhere.flac")),
            "a traversal out of the root is still out of the root"
        );
        assert!(!inside_a_root(&config, &library), "the root is not a track");

        // A server source owns no local files, so nothing is deletable.
        let server = config::AppConfig {
            active_source: config::Source::Server("s".into()),
            ..config
        };
        assert!(!inside_a_root(&server, &inside));
    }

    #[test]
    fn artwork_must_decode_before_it_is_stored() {
        assert!(validate_image(b"").is_err(), "empty");
        assert!(
            validate_image(b"\x89PNG\r\n\x1a\nnot an image").is_err(),
            "png magic alone is not a picture"
        );
    }
}
