use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use config::{MusicService, Source};
use db::Db;
use futures_util::StreamExt;
use lofty::file::TaggedFileExt;
use sha2::{Digest, Sha256};

use super::{
    AlbumType, ArtistView, AuthOutcome, Capabilities, FavoritesSync, LibrarySnapshot, MediaSource,
    PlaylistOps, RadioSeeds, SourceError, StreamInfo,
};
use crate::server_ops::ServerConn;
use crate::smb::{Location, SmbStream};

pub(super) struct SmbSource {
    db: Db,
    source: Source,
    location: Result<Location, SourceError>,
    username: String,
    password: String,
}

impl SmbSource {
    pub(super) fn new(db: Db, source: Source, conn: &ServerConn) -> Self {
        Self {
            db,
            source,
            location: Location::parse(&conn.url),
            username: conn.user_id.clone(),
            password: conn.token.clone(),
        }
    }

    async fn connect(&self) -> Result<crate::smb::Session, SourceError> {
        self.location
            .as_ref()
            .map_err(Clone::clone)?
            .connect(&self.username, &self.password)
            .await
    }
}

#[async_trait]
impl MediaSource for SmbSource {
    fn source(&self) -> &Source {
        &self.source
    }
    fn db(&self) -> &Db {
        &self.db
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            edit_tags: false,
            delete_from_disk: false,
            scan_folders: false,
            folders: false,
            browse_folders: false,
            external_devices: false,
            browser_playback: false,
            sync: true,
            downloads: false,
            uploads: false,
            storage_quota: false,
            discover: false,
            dont_recommend: false,
            radio: RadioSeeds::NONE,
            playlists: PlaylistOps::None,
            artist_view: ArtistView::Library,
            albums: AlbumType::Standard,
            favorites_sync: FavoritesSync::Instant,
        }
    }

    async fn fetch_library(&self) -> Result<LibrarySnapshot, SourceError> {
        self.fetch_library_with_progress(Arc::new(|_| {})).await
    }

    async fn fetch_library_with_progress(
        &self,
        on_progress: Arc<dyn Fn(String) + Send + Sync>,
    ) -> Result<LibrarySnapshot, SourceError> {
        let mut session = self.connect().await?;
        let mut pending = vec![(String::new(), 0)];
        let mut paths = Vec::new();
        let mut entries = 0;
        while let Some((directory, depth)) = pending.pop() {
            if depth > 64 {
                return Err(SourceError::Backend(
                    "SMB directory nesting exceeds 64 levels".into(),
                ));
            }
            for entry in session.list(&directory).await? {
                entries += 1;
                if entries > 100_000 {
                    return Err(SourceError::Backend(
                        "SMB library exceeds 100,000 entries; choose a smaller subfolder".into(),
                    ));
                }
                let path = if directory.is_empty() {
                    entry.name
                } else {
                    format!("{directory}/{}", entry.name)
                };
                if entry.is_directory {
                    pending.push((path, depth + 1));
                } else if reader::scanner::is_audio_file(Path::new(&path)) {
                    paths.push(path);
                }
            }
        }
        paths.sort_unstable();
        let mut library = reader::Library {
            tracks: self.db.tracks_by_keys(&self.source, &paths).await?,
            albums: self.db.albums(&self.source).await?,
            ..Default::default()
        };
        let missing = scan_candidates(paths, &library);
        tracing::info!(
            new_files = missing.len(),
            existing = library.tracks.len(),
            "scanning SMB library"
        );
        let session = tokio::sync::Mutex::new(session);
        let mut scans = futures_util::stream::iter(missing)
            .map(|path| {
                let session = &session;
                async move {
                    let stream = session.lock().await.open(&path).await?;
                    tokio::task::spawn_blocking(move || {
                        scan_track(stream, &path).map(|scanned| (path, scanned))
                    })
                    .await
                    .map_err(|_| SourceError::Backend("SMB metadata reader failed".into()))?
                }
            })
            .buffered(4);
        let mut scanned = Vec::new();
        while let Some(result) = scans.next().await {
            let (path, track) = result?;
            if let Some(name) = Path::new(&path).file_name() {
                on_progress(name.to_string_lossy().into_owned());
            }
            scanned.extend(track);
        }
        reader::scanner::merge_scanned_tracks(&mut library, scanned);
        let album_ids: HashSet<_> = library.tracks.iter().map(|track| &track.album_id).collect();
        library.albums.retain(|album| album_ids.contains(&album.id));
        let covers: HashMap<_, _> = library
            .albums
            .iter()
            .filter_map(|album| {
                album
                    .cover_path
                    .as_ref()
                    .map(|path| (&album.id, path.to_string_lossy().into_owned()))
            })
            .collect();
        for track in &mut library.tracks {
            if let Some(cover) = covers.get(&track.album_id) {
                track.cover = Some(cover.clone());
            }
        }
        tracing::info!(
            total_tracks = library.tracks.len(),
            "SMB library scan complete"
        );
        Ok(LibrarySnapshot {
            albums: library.albums,
            tracks: library.tracks,
            artist_images: Vec::new(),
        })
    }

    async fn open_stream(
        &self,
        item_id: &str,
        progress: Option<crate::stream::stream_buffer::BufferProgressCallback>,
    ) -> Result<Option<Box<dyn symphonia::core::io::MediaSource>>, SourceError> {
        let mut stream = self.connect().await?.open(item_id).await?;
        stream.set_progress(progress);
        Ok(Some(Box::new(stream)))
    }

    async fn fetch_missing_covers(
        &self,
        albums: &[reader::Album],
        tracks: &[reader::Track],
        on_progress: Arc<dyn Fn(String) + Send + Sync>,
    ) -> Result<Vec<reader::Album>, SourceError> {
        let candidates = cover_candidates(albums, tracks);
        if candidates.is_empty() {
            return Ok(Vec::new());
        }
        let session = tokio::sync::Mutex::new(self.connect().await?);
        let mut scans = futures_util::stream::iter(candidates)
            .map(|(mut album, path)| {
                let session = &session;
                async move {
                    let mut stream = session.lock().await.open(&path).await?;
                    stream.limit_reads(32 * 1024 * 1024);
                    let cover = tokio::task::spawn_blocking(move || {
                        let tagged = lofty::probe::Probe::new(stream)
                            .options(lofty::config::ParseOptions::new().read_properties(false))
                            .guess_file_type()
                            .map_err(lofty::error::LoftyError::from)
                            .and_then(|probe| probe.read())
                            .inspect_err(
                                |error| tracing::debug!(path, %error, "SMB cover unavailable"),
                            )
                            .ok()?;
                        let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
                        reader::metadata::extract_embedded_cover(&tagged, tag)
                            .and_then(cache_picture)
                    })
                    .await
                    .map_err(|_| SourceError::Backend("SMB cover reader failed".into()))?;
                    album.cover_path = cover;
                    Ok::<_, SourceError>(album)
                }
            })
            .buffered(4);
        let mut resolved = Vec::new();
        while let Some(result) = scans.next().await {
            let album = result?;
            on_progress(album.title.clone());
            if album.cover_path.is_some() {
                resolved.push(album);
            }
        }
        Ok(resolved)
    }

    async fn resolve_stream(&self, _: &str) -> Result<StreamInfo, SourceError> {
        Err(SourceError::unsupported("SMB stream URLs"))
    }

    async fn validate(&self) -> AuthOutcome {
        let result = async { self.connect().await?.list("").await }.await;
        match result {
            Ok(_) => AuthOutcome::Valid,
            Err(SourceError::Auth) => AuthOutcome::Expired,
            Err(_) => AuthOutcome::Unreachable,
        }
    }

    async fn fetch_favorites(&self) -> Result<Vec<String>, SourceError> {
        self.db
            .favorites(self.source.as_str())
            .await
            .map_err(SourceError::from)
    }

    async fn push_favorite(&self, _: &str, _: bool) -> Result<(), SourceError> {
        Ok(())
    }

    async fn add_to_playlist(&self, _: &str, _: &[String]) -> Result<Vec<String>, SourceError> {
        Err(SourceError::unsupported("playlists"))
    }

    async fn create_playlist(&self, _: &str, _: &[String]) -> Result<String, SourceError> {
        Err(SourceError::unsupported("playlists"))
    }

    async fn remove_from_playlist(
        &self,
        _: &str,
        _: &reader::Track,
        _: usize,
    ) -> Result<(), SourceError> {
        Err(SourceError::unsupported("playlists"))
    }
}

fn scan_candidates(paths: Vec<String>, library: &reader::Library) -> Vec<String> {
    let albums: HashSet<_> = library.albums.iter().map(|album| &album.id).collect();
    let existing: HashSet<_> = library
        .tracks
        .iter()
        .filter(|track| {
            track.duration > 0
                && albums.contains(&track.album_id)
                && track
                    .album_id
                    .strip_prefix("smb:")
                    .is_some_and(reader::metadata::album_id_is_current)
        })
        .map(|track| track.id.key().into_owned())
        .collect();
    paths
        .into_iter()
        .filter(|path| !existing.contains(path))
        .collect()
}

fn cover_candidates(
    albums: &[reader::Album],
    tracks: &[reader::Track],
) -> Vec<(reader::Album, String)> {
    let missing = reader::missing_cover_ids(&reader::Library {
        albums: albums.to_vec(),
        ..Default::default()
    });
    let mut representatives = HashMap::new();
    for track in tracks {
        representatives
            .entry(&track.album_id)
            .or_insert(track.id.key());
    }
    albums
        .iter()
        .filter(|album| missing.contains(&album.id))
        .filter_map(|album| {
            representatives
                .get(&album.id)
                .map(|path| (album.clone(), path.to_string()))
        })
        .collect()
}

fn scan_track(
    mut stream: SmbStream,
    path: &str,
) -> Result<Option<reader::ScannedTrack>, SourceError> {
    stream.limit_reads(8 * 1024 * 1024);
    scan_stream(stream, path)
}

fn scan_stream(
    stream: impl symphonia::core::io::MediaSource + 'static,
    path: &str,
) -> Result<Option<reader::ScannedTrack>, SourceError> {
    if Path::new(path)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("mka"))
    {
        return match reader::metadata::read_with_symphonia(Box::new(stream), Path::new(path)) {
            Ok(scanned) => Ok(Some(remote_metadata(scanned, path))),
            Err(symphonia::core::errors::Error::IoError(error)) => {
                metadata_error(path, error.into())
            }
            Err(error) => {
                tracing::warn!(path, %error, "skipping unreadable SMB audio file");
                Ok(None)
            }
        };
    }
    let size = stream.byte_len().unwrap_or(0);
    match lofty::probe::Probe::new(stream)
        .options(lofty::config::ParseOptions::new().read_cover_art(false))
        .guess_file_type()
        .map_err(lofty::error::LoftyError::from)
        .and_then(|probe| probe.read())
    {
        Ok(tagged) => Ok(Some(metadata(path, &tagged, size))),
        Err(error) => metadata_error(path, error),
    }
}

fn metadata_error(
    path: &str,
    error: lofty::error::LoftyError,
) -> Result<Option<reader::ScannedTrack>, SourceError> {
    if let lofty::error::ErrorKind::Io(io) = error.kind()
        && let Some(source) = io
            .get_ref()
            .and_then(|error| error.downcast_ref::<SourceError>())
    {
        return Err(source.clone());
    }
    tracing::warn!(path, %error, "skipping unreadable SMB audio file");
    Ok(None)
}

fn metadata(path: &str, tagged: &lofty::file::TaggedFile, size: u64) -> reader::ScannedTrack {
    let scanned = reader::metadata::from_tagged_file(tagged, Path::new(path), size);
    remote_metadata(scanned, path)
}

fn remote_metadata(mut scanned: reader::ScannedTrack, path: &str) -> reader::ScannedTrack {
    scanned.track.id = reader::TrackId::Server {
        service: MusicService::Smb,
        item_id: path.to_string(),
    };
    scanned.track.album_id =
        reader::CoverRef::stored_item_ref(MusicService::Smb, &scanned.track.album_id, None);
    scanned.track.cover = Some(reader::CoverRef::NO_COVER.to_string());
    scanned.album.id = scanned.track.album_id.clone();
    scanned
}

fn cache_picture(picture: &lofty::picture::Picture) -> Option<PathBuf> {
    #[cfg(target_os = "android")]
    let dir = db::config_dir().join("cache/smb-covers");
    #[cfg(not(target_os = "android"))]
    let dir = directories::ProjectDirs::from("moe", "kopuz", "kopuz")?
        .cache_dir()
        .join("smb-covers");
    let extension = picture.mime_type()?.ext()?;
    let path = dir.join(format!(
        "{}.{extension}",
        hex::encode(Sha256::digest(picture.data()))
    ));
    if !path.is_file() {
        std::fs::create_dir_all(&dir).ok()?;
        let staging = dir.join(format!("{}.part", uuid::Uuid::new_v4()));
        if std::fs::write(&staging, picture.data())
            .and_then(|()| std::fs::rename(&staging, &path))
            .is_err()
        {
            let _ = std::fs::remove_file(staging);
            return None;
        }
    }
    Some(path)
}

#[cfg(test)]
mod tests;
