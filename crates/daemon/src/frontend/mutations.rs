use std::path::Path;

use super::*;

impl FrontendService {
    pub async fn update_track_metadata(
        &self,
        patch: api::TrackMetadataPatch,
    ) -> Result<api::TrackInfo, ApiError> {
        let config = self.current().await;
        let mut track = self
            .db
            .tracks_by_keys(&config.active_source, std::slice::from_ref(&patch.key))
            .await
            .map_err(Self::db_error)?
            .into_iter()
            .next()
            .ok_or_else(|| ApiError::not_found("track not found"))?;
        let path = track
            .id
            .local_path()
            .map(Path::to_owned)
            .ok_or_else(|| ApiError::unsupported("only local track tags are editable"))?;
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
        let edits = reader::TrackEdits {
            title: title.clone(),
            artist: artist.clone(),
            album: album.clone(),
            track_number,
            disc_number,
            cover: reader::CoverChange::Keep,
        };
        tokio::task::spawn_blocking(move || reader::write_tags(&path, &edits))
            .await
            .map_err(|error| ApiError::internal(format!("tag writer task failed: {error}")))?
            .map_err(ApiError::internal)?;
        track.title = title.trim().to_string();
        track.artist = artist.trim().to_string();
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
            .map_err(Self::db_error)?;
        self.library.invalidate(api::Table::Tracks);
        Ok(crate::wire::track_info(&track, &config))
    }

    pub(super) fn allowed_local_path(config: &config::AppConfig, path: &Path) -> bool {
        let Ok(path) = path.canonicalize() else {
            return false;
        };
        config
            .music_directory
            .iter()
            .chain(
                config
                    .local_sources
                    .iter()
                    .flat_map(|source| source.directories.iter()),
            )
            .filter_map(|root| root.canonicalize().ok())
            .any(|root| path.starts_with(root))
    }

    pub async fn delete_tracks(&self, keys: &[String], from_disk: bool) -> Result<(), ApiError> {
        let config = self.current().await;
        let tracks = self
            .db
            .tracks_by_keys(&config.active_source, keys)
            .await
            .map_err(Self::db_error)?;
        if from_disk {
            let mut paths = Vec::with_capacity(tracks.len());
            for track in &tracks {
                let path = track.id.local_path().ok_or_else(|| {
                    ApiError::unsupported("server tracks cannot be deleted from local storage")
                })?;
                if !Self::allowed_local_path(&config, path) {
                    return Err(ApiError::invalid_input(
                        "track path is outside configured library roots",
                    ));
                }
                paths.push(path.to_owned());
            }
            tokio::task::spawn_blocking(move || {
                for path in paths {
                    if let Err(error) = std::fs::remove_file(path)
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
            .await
            .delete_tracks(keys)
            .await
            .map_err(Self::source_error)?;
        self.library.invalidate(api::Table::Tracks);
        Ok(())
    }

    pub async fn delete_album(&self, id: &str, from_disk: bool) -> Result<(), ApiError> {
        let config = self.current().await;
        let tracks = self
            .db
            .album_tracks(&config.active_source, id)
            .await
            .map_err(Self::db_error)?;
        let keys: Vec<String> = tracks
            .iter()
            .map(|track| track.id.key().into_owned())
            .collect();
        if from_disk {
            self.delete_tracks(&keys, true).await?;
        }
        self.source()
            .await
            .delete_album(id)
            .await
            .map_err(Self::source_error)?;
        self.library.invalidate(api::Table::Albums);
        Ok(())
    }

    pub async fn upload_artwork(&self, upload: api::ArtworkUpload) -> Result<(), ApiError> {
        const MAX_BYTES: usize = 32 * 1024 * 1024;
        if upload.data.is_empty() || upload.data.len() > MAX_BYTES {
            return Err(ApiError::invalid_input(
                "artwork must be between 1 byte and 32 MiB",
            ));
        }
        let mut image = image::ImageReader::new(std::io::Cursor::new(&upload.data))
            .with_guessed_format()
            .map_err(|error| ApiError::invalid_input(format!("invalid image: {error}")))?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(8192);
        limits.max_image_height = Some(8192);
        limits.max_alloc = Some(128 * 1024 * 1024);
        image.limits(limits);
        image
            .decode()
            .map_err(|error| ApiError::invalid_input(format!("invalid image: {error}")))?;
        let target = upload
            .target
            .ok_or_else(|| ApiError::invalid_input("artwork target is required"))?;
        if let api::ArtworkTarget::Track { key } = target {
            return self
                .update_track_artwork(&key, reader::CoverChange::Set(upload.data))
                .await;
        }
        let config = self.current().await;
        let previous = match &target {
            api::ArtworkTarget::Track { .. } => None,
            api::ArtworkTarget::Album { id } => {
                self.db
                    .album(&config.active_source, id)
                    .await
                    .map_err(Self::db_error)?
                    .ok_or_else(|| ApiError::not_found("album not found"))?
                    .cover_path
            }
            api::ArtworkTarget::Artist { name } => self
                .db
                .artist_images()
                .await
                .map_err(Self::db_error)?
                .0
                .get(&utils::artist::normalize_artist_key(name))
                .cloned(),
            api::ArtworkTarget::Playlist { id } => self.playlist(&config, id).await?.cover_path,
        };
        let extension = match upload.content_type.as_str() {
            "image/jpeg" => "jpg",
            "image/png" => "png",
            "image/webp" => "webp",
            _ => return Err(ApiError::invalid_input("unsupported artwork content type")),
        };
        let path = self
            .uploads
            .join(format!("{}.{extension}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&self.uploads)
            .map_err(|error| ApiError::internal(format!("artwork directory failed: {error}")))?;
        tokio::fs::write(&path, &upload.data)
            .await
            .map_err(|error| ApiError::internal(format!("artwork write failed: {error}")))?;
        let path_string = path.to_string_lossy().into_owned();
        let result: Result<(), ApiError> = match target {
            api::ArtworkTarget::Track { .. } => unreachable!(),
            api::ArtworkTarget::Album { id } => {
                async {
                    self.source()
                        .await
                        .update_album_cover(&id, Some(&path_string), true)
                        .await
                        .map_err(Self::source_error)?;
                    self.library.invalidate(api::Table::Albums);
                    Ok(())
                }
                .await
            }
            api::ArtworkTarget::Artist { name } => {
                async {
                    self.source()
                        .await
                        .set_artist_image(
                            &utils::artist::normalize_artist_key(&name),
                            "custom",
                            Some(&path_string),
                        )
                        .await
                        .map_err(Self::source_error)?;
                    self.library.invalidate(api::Table::Tracks);
                    Ok(())
                }
                .await
            }
            api::ArtworkTarget::Playlist { id } => {
                async {
                    let config = self.current().await;
                    let playlist = self.playlist(&config, &id).await?;
                    self.source()
                        .await
                        .set_playlist_cover(
                            &id,
                            &playlist.name,
                            &path,
                            playlist.image_tag.as_deref(),
                        )
                        .await
                        .map_err(Self::source_error)?;
                    self.library.invalidate(api::Table::Playlists);
                    Ok(())
                }
                .await
            }
        };
        if result.is_err() {
            let _ = tokio::fs::remove_file(path).await;
        } else if let Some(previous) = previous
            && previous.starts_with(&self.uploads)
            && previous != path
        {
            let _ = tokio::fs::remove_file(previous).await;
        }
        result
    }

    pub async fn remove_artwork(&self, target: api::ArtworkTarget) -> Result<(), ApiError> {
        let config = self.current().await;
        let previous = match target {
            api::ArtworkTarget::Track { key } => {
                self.update_track_artwork(&key, reader::CoverChange::Remove)
                    .await?;
                None
            }
            api::ArtworkTarget::Album { id } => {
                let album = self
                    .db
                    .album(&config.active_source, &id)
                    .await
                    .map_err(Self::db_error)?
                    .ok_or_else(|| ApiError::not_found("album not found"))?;
                self.source()
                    .await
                    .update_album_cover(&id, None, false)
                    .await
                    .map_err(Self::source_error)?;
                self.library.invalidate(api::Table::Albums);
                album.cover_path
            }
            api::ArtworkTarget::Artist { name } => {
                let normalized = utils::artist::normalize_artist_key(&name);
                let previous = self
                    .db
                    .artist_images()
                    .await
                    .map_err(Self::db_error)?
                    .0
                    .get(&normalized)
                    .cloned();
                self.source()
                    .await
                    .set_artist_image(&normalized, "custom", None)
                    .await
                    .map_err(Self::source_error)?;
                self.library.invalidate(api::Table::Tracks);
                previous
            }
            api::ArtworkTarget::Playlist { id } => {
                let playlist = self.playlist(&config, &id).await?;
                let previous = playlist.cover_path.clone();
                self.db
                    .upsert_playlist_meta(
                        &config.active_source,
                        &id,
                        &playlist.name,
                        None,
                        playlist.image_tag.as_deref(),
                    )
                    .await
                    .map_err(Self::db_error)?;
                self.library.invalidate(api::Table::Playlists);
                previous
            }
        };
        if let Some(path) = previous
            && path.starts_with(&self.uploads)
        {
            let _ = tokio::fs::remove_file(path).await;
        }
        Ok(())
    }

    async fn update_track_artwork(
        &self,
        key: &str,
        cover: reader::CoverChange,
    ) -> Result<(), ApiError> {
        let config = self.current().await;
        let track = self
            .db
            .tracks_by_keys(&config.active_source, &[key.to_string()])
            .await
            .map_err(Self::db_error)?
            .into_iter()
            .next()
            .ok_or_else(|| ApiError::not_found("track not found"))?;
        let path = track
            .id
            .local_path()
            .map(Path::to_owned)
            .ok_or_else(|| ApiError::unsupported("only local track artwork is editable"))?;
        if !Self::allowed_local_path(&config, &path) {
            return Err(ApiError::invalid_input(
                "track path is outside configured library roots",
            ));
        }
        let edits = reader::TrackEdits {
            title: track.title,
            artist: track.artist,
            album: track.album,
            track_number: track.track_number,
            disc_number: track.disc_number,
            cover,
        };
        tokio::task::spawn_blocking(move || reader::write_tags(&path, &edits))
            .await
            .map_err(|error| ApiError::internal(format!("tag writer task failed: {error}")))?
            .map_err(ApiError::internal)?;
        self.library.invalidate(api::Table::Tracks);
        Ok(())
    }
}
