//! The library's long-running jobs: the local filesystem scan and the
//! remote library pull, both cooperative with the job runner.

use super::*;

impl LibraryService {
    pub fn spawn_scan(self: &Arc<Self>, runner: &JobRunner) -> Result<JobRef, ApiError> {
        let service = self.clone();
        runner.start(JobKind::Scan, move |ctx| async move {
            let config = service.current_config();
            service.run_scan(&ctx, &config).await
        })
    }

    /// Like [`Self::spawn_scan`], but with the roots pinned by the caller.
    /// The embedded frontend uses this: its config signal is the authority,
    /// and reading the session watch here instead would race the async
    /// config push (a lost race scans and prunes against default roots).
    pub fn spawn_scan_with_config(
        self: &Arc<Self>,
        runner: &JobRunner,
        config: config::AppConfig,
    ) -> Result<JobRef, ApiError> {
        let service = self.clone();
        runner.start(JobKind::Scan, move |ctx| async move {
            service.run_scan(&ctx, &config).await
        })
    }

    pub fn spawn_remote_sync(self: &Arc<Self>, runner: &JobRunner) -> Result<JobRef, ApiError> {
        let service = self.clone();
        runner.start(JobKind::LibrarySync, move |ctx| async move {
            let config = service.current_config();
            service.run_remote_sync(&ctx, &config).await
        })
    }

    /// Local filesystem scan, ported from the app's rescan effect: DB-seeded
    /// working set, per-root scan, retain-by-root, chunked upserts, prune,
    /// local artist images with self-heal, then cover indexing and (when
    /// enabled) network cover fetching. The job runner's single-flight
    /// replaces the app's epoch supersession; cancellation is checked between
    /// phases and chunks.
    pub async fn run_scan(&self, ctx: &JobCtx, config: &config::AppConfig) -> Result<(), ApiError> {
        let db_error = |error: db::DbError| ApiError::internal(format!("database error: {error}"));
        for (source, configured_dirs) in Self::scan_roots(config) {
            if ctx.cancelled() {
                return Ok(());
            }
            let scannable_dirs: Vec<PathBuf> = configured_dirs
                .iter()
                .filter(|dir| dir.exists())
                .cloned()
                .collect();

            if configured_dirs.is_empty() {
                self.db
                    .prune_source(&source, &[], &[])
                    .await
                    .map_err(db_error)?;
                self.invalidate(Table::Tracks);
                self.invalidate(Table::Albums);
                continue;
            }

            ctx.progress("seeding", None, None, None);
            let mut seed_tracks: Vec<Track> = Vec::new();
            let mut seen_keys = HashSet::new();
            for dir in &configured_dirs {
                let mut prefix = dir.to_string_lossy().into_owned();
                if !prefix.ends_with(std::path::MAIN_SEPARATOR) {
                    prefix.push(std::path::MAIN_SEPARATOR);
                }
                let found = self
                    .db
                    .folder_tracks(&source, &prefix)
                    .await
                    .map_err(db_error)?;
                for track in found {
                    if seen_keys.insert(track.id.key().into_owned()) {
                        seed_tracks.push(track);
                    }
                }
            }
            let seed_albums = self.db.albums(&source).await.map_err(db_error)?;
            let mut library = reader::Library {
                root_paths: configured_dirs.clone(),
                tracks: seed_tracks,
                albums: seed_albums,
                ..Default::default()
            };

            let progress_ctx = ctx.clone();
            let progress: Arc<dyn Fn(String) + Send + Sync> = Arc::new(move |file: String| {
                progress_ctx.progress_throttled("scanning", Some(file));
            });
            for dir in &scannable_dirs {
                if ctx.cancelled() {
                    return Ok(());
                }
                let _ = reader::scan_directory(
                    dir.clone(),
                    self.cover_cache.clone(),
                    &mut library,
                    progress.clone(),
                )
                .await;
            }

            library.tracks.retain(|track| {
                let in_configured_root = configured_dirs.iter().any(|dir| {
                    track
                        .id
                        .local_path()
                        .is_some_and(|path| path.starts_with(dir))
                });
                let in_scannable_root = scannable_dirs.iter().any(|dir| {
                    track
                        .id
                        .local_path()
                        .is_some_and(|path| path.starts_with(dir))
                });
                in_configured_root
                    && (!in_scannable_root
                        || track.id.local_path().is_some_and(|path| path.exists()))
            });
            let valid_album_ids: HashSet<_> = library
                .tracks
                .iter()
                .map(|track| track.album_id.clone())
                .collect();
            library
                .albums
                .retain(|album| valid_album_ids.contains(&album.id));

            let total = library.tracks.len() as u64;
            let mut done = 0u64;
            for chunk in library.tracks.chunks(100) {
                if ctx.cancelled() {
                    return Ok(());
                }
                self.db
                    .upsert_tracks(&source, chunk)
                    .await
                    .map_err(db_error)?;
                done += chunk.len() as u64;
                ctx.progress("persisting", Some(done), Some(total), None);
                self.invalidate(Table::Tracks);
            }
            self.db
                .upsert_albums(&source, &library.albums)
                .await
                .map_err(db_error)?;
            let keep_keys: Vec<String> = library
                .tracks
                .iter()
                .map(|track| track.id.key().into_owned())
                .collect();
            let keep_albums: Vec<String> = library
                .albums
                .iter()
                .map(|album| album.id.clone())
                .collect();
            self.db
                .prune_source(&source, &keep_keys, &keep_albums)
                .await
                .map_err(db_error)?;
            for (artist, image) in &library.local_artist_images {
                let path = image.to_string_lossy().into_owned();
                let _ = self.db.set_artist_image(artist, "local", Some(&path)).await;
            }
            if let Ok((_, photos)) = self.db.artist_images().await {
                for (artist, photo) in photos {
                    if let reader::ArtistImageRef::Local(path) = photo
                        && !path.exists()
                    {
                        let _ = self.db.set_artist_image(&artist, "local", None).await;
                    }
                }
            }
            self.invalidate(Table::Tracks);
            self.invalidate(Table::Albums);

            ctx.progress("indexing covers", None, None, None);
            let missing_local = reader::missing_cover_ids(&library);
            let _ = reader::index_local_covers(
                &mut library,
                self.cover_cache.clone(),
                progress.clone(),
            )
            .await;
            self.persist_resolved_covers(ctx, &source, &library.albums, &missing_local)
                .await;

            if config.auto_fetch_covers && !ctx.cancelled() {
                ctx.progress("fetching covers", None, None, None);
                let lastfm_key = {
                    let key = config.lastfm_api_key.trim().to_owned();
                    (!key.is_empty()).then_some(key)
                };
                let fetcher = reader::cover_fetcher::CoverFetcher::new(
                    self.cover_cache.clone(),
                    config.cover_fetch_strategy,
                    lastfm_key,
                    progress.clone(),
                );
                let missing_before = reader::missing_cover_ids(&library);
                let _ = fetcher.fetch_missing_covers(&mut library).await;
                self.persist_resolved_covers(ctx, &source, &library.albums, &missing_before)
                    .await;
            }
        }
        Ok(())
    }

    async fn persist_resolved_covers(
        &self,
        ctx: &JobCtx,
        source: &config::Source,
        albums: &[reader::Album],
        missing_ids: &HashSet<String>,
    ) {
        let mut changed = false;
        for album in albums {
            if ctx.cancelled() {
                break;
            }
            if !missing_ids.contains(&album.id) {
                continue;
            }
            let Some(cover) = album.cover_path.as_ref() else {
                continue;
            };
            let path = cover.to_string_lossy().into_owned();
            match self
                .db
                .update_album_cover_if_not_manual(source, &album.id, &path)
                .await
            {
                Ok(written) => changed |= written,
                Err(error) => {
                    tracing::warn!(album_id = %album.id, %error, "cover persist failed");
                }
            }
        }
        if changed {
            self.invalidate(Table::Albums);
        }
    }

    /// Remote library pull, ported from `sync_server_library`: fetch the
    /// snapshot, merge manual covers, chunked upserts with invalidations,
    /// artist images, then prune what the server dropped.
    pub async fn run_remote_sync(
        &self,
        ctx: &JobCtx,
        config: &config::AppConfig,
    ) -> Result<(), ApiError> {
        let source: server::source::ActiveSource =
            Arc::from(server::source::active(self.db.clone(), config));
        if !source.capabilities().sync {
            return Err(ApiError::unsupported(
                "the active source has no library sync",
            ));
        }
        let src = source.source().clone();
        let existing_albums = self
            .db
            .albums(&src)
            .await
            .map_err(|error| ApiError::internal(format!("database error: {error}")))?;
        let existing_albums_by_id: HashMap<String, &reader::Album> = existing_albums
            .iter()
            .map(|album| (normalize_album_id(&album.id), album))
            .collect();
        let merge_cover = |mut album: reader::Album| -> reader::Album {
            if let Some(old) = existing_albums_by_id.get(&normalize_album_id(&album.id)) {
                if album.cover_path.is_none() || old.manual_cover {
                    album.cover_path = old.cover_path.clone();
                }
                if old.manual_cover {
                    album.manual_cover = true;
                }
            }
            album
        };

        ctx.progress("fetching library", None, None, None);
        let snapshot = source
            .fetch_library()
            .await
            .map_err(|error| ApiError::internal(error.to_string()))?;

        let merged_albums: Vec<reader::Album> =
            snapshot.albums.into_iter().map(merge_cover).collect();
        let total = (merged_albums.len() + snapshot.tracks.len()) as u64;
        let mut done = 0u64;
        for chunk in merged_albums.chunks(100) {
            if ctx.cancelled() {
                return Ok(());
            }
            source
                .upsert_albums(chunk)
                .await
                .map_err(|error| ApiError::internal(error.to_string()))?;
            done += chunk.len() as u64;
            ctx.progress("persisting", Some(done), Some(total), None);
            self.invalidate(Table::Albums);
        }
        for chunk in snapshot.tracks.chunks(100) {
            if ctx.cancelled() {
                return Ok(());
            }
            source
                .upsert_tracks(chunk)
                .await
                .map_err(|error| ApiError::internal(error.to_string()))?;
            done += chunk.len() as u64;
            ctx.progress("persisting", Some(done), Some(total), None);
            self.invalidate(Table::Tracks);
        }
        for (name, url) in &snapshot.artist_images {
            let _ = source.set_artist_image(name, "server", Some(url)).await;
        }
        let keep_keys: Vec<String> = snapshot
            .tracks
            .iter()
            .map(|track| track.id.key().into_owned())
            .collect();
        let keep_albums: Vec<String> = merged_albums.iter().map(|album| album.id.clone()).collect();
        let _ = source.prune(&keep_keys, &keep_albums).await;
        self.invalidate(Table::Tracks);
        self.invalidate(Table::Albums);
        Ok(())
    }
}
