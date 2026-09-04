use super::*;
use futures_util::{StreamExt, stream};

impl FrontendService {
    pub async fn albums(
        &self,
        filter: api::AlbumFilter,
        page: api::Page,
    ) -> Result<api::AlbumPage, ApiError> {
        let config = self.current().await;
        let mut albums = self
            .db
            .albums(&config.active_source)
            .await
            .map_err(Self::db_error)?;
        if let Some(search) = filter
            .search
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            let search = search.to_lowercase();
            albums.retain(|album| {
                album.title.to_lowercase().contains(&search)
                    || album.artist.to_lowercase().contains(&search)
            });
        }
        if let Some(artist) = filter.artist.as_deref() {
            albums.retain(|album| album.artist.eq_ignore_ascii_case(artist));
        }
        if let Some(genre) = filter.genre.as_deref() {
            albums.retain(|album| {
                album
                    .genre
                    .split(['/', ';', ','])
                    .any(|value| value.trim().eq_ignore_ascii_case(genre))
            });
        }
        match filter.sort.as_deref() {
            Some("title") => albums.sort_by_key(|album| album.title.to_lowercase()),
            Some("year") => albums.sort_by_key(|album| album.year),
            Some("genre") => albums.sort_by_key(|album| album.genre.to_lowercase()),
            _ => albums
                .sort_by_key(|album| (album.artist.to_lowercase(), album.title.to_lowercase())),
        }
        let total = albums.len().min(u32::MAX as usize) as u32;
        let items = albums
            .iter()
            .skip(page.offset as usize)
            .take(page.limit as usize)
            .map(Self::album_info)
            .collect();
        Ok(api::AlbumPage {
            total,
            offset: page.offset,
            items,
        })
    }

    pub async fn album(&self, id: &str) -> Result<api::AlbumInfo, ApiError> {
        let config = self.current().await;
        self.db
            .album(&config.active_source, id)
            .await
            .map_err(Self::db_error)?
            .as_ref()
            .map(Self::album_info)
            .ok_or_else(|| ApiError::not_found("album not found"))
    }

    pub async fn artists(&self, page: api::Page) -> Result<api::ArtistPage, ApiError> {
        let config = self.current().await;
        let artists = self
            .db
            .artists(&config.active_source)
            .await
            .map_err(Self::db_error)?;
        let albums = self
            .db
            .albums(&config.active_source)
            .await
            .map_err(Self::db_error)?;
        let images = self.db.artist_images().await.map_err(Self::db_error)?;
        let total = artists.len().min(u32::MAX as usize) as u32;
        let items = artists
            .iter()
            .skip(page.offset as usize)
            .take(page.limit as usize)
            .map(|(name, count)| {
                let normalized = utils::artist::normalize_artist_key(name);
                api::ArtistInfo {
                    name: name.clone(),
                    track_count: *count,
                    album_count: albums
                        .iter()
                        .filter(|album| album.artist.eq_ignore_ascii_case(name))
                        .count()
                        .min(u32::MAX as usize) as u32,
                    artwork: (images.0.contains_key(&normalized)
                        || images.1.contains_key(&normalized))
                    .then(|| name.clone()),
                    manual_artwork: images.0.contains_key(&normalized),
                }
            })
            .collect();
        Ok(api::ArtistPage {
            total,
            offset: page.offset,
            items,
        })
    }

    pub async fn refresh_artist_artwork(
        &self,
        names: Vec<String>,
    ) -> Result<Vec<String>, ApiError> {
        const MISS_KIND: &str = "artist_photo_miss";
        const MISS_TTL_SECS: i64 = 86_400;

        let mut names: Vec<String> = names
            .into_iter()
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty())
            .collect();
        names.sort_by_key(|name| name.to_lowercase());
        names.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
        let source = self.source().await;
        let mut changed = false;
        match source.capabilities().artist_view {
            server::source::ArtistView::Library if source.capabilities().sync => {
                let images = source
                    .fetch_artist_images()
                    .await
                    .map_err(Self::source_error)?;
                for (name, url) in images {
                    source
                        .set_artist_image(
                            &utils::artist::normalize_artist_key(&name),
                            "server",
                            Some(&url),
                        )
                        .await
                        .map_err(Self::source_error)?;
                    changed = true;
                }
            }
            server::source::ArtistView::Remote => {
                let fresh_misses: std::collections::HashSet<String> = self
                    .db
                    .meta_keys_since(MISS_KIND, MISS_TTL_SECS)
                    .await
                    .map_err(Self::db_error)?
                    .into_iter()
                    .collect();
                let images = self.db.artist_images().await.map_err(Self::db_error)?;
                let pending: Vec<String> = names
                    .iter()
                    .filter(|name| {
                        let normalized = utils::artist::normalize_artist_key(name);
                        !images.0.contains_key(&normalized)
                            && !images.1.contains_key(&normalized)
                            && !fresh_misses.contains(&normalized)
                    })
                    .cloned()
                    .collect();
                let outcomes: Vec<bool> = stream::iter(pending.into_iter().map(|name| {
                    let source = source.clone();
                    async move {
                        let normalized = utils::artist::normalize_artist_key(&name);
                        match source.fetch_artist_image(&name).await {
                            Ok(Some(url)) => source
                                .set_artist_image(&normalized, "server", Some(&url))
                                .await
                                .is_ok(),
                            Ok(None) => {
                                let _ = source.set_meta(&normalized, MISS_KIND, "").await;
                                false
                            }
                            Err(_) => false,
                        }
                    }
                }))
                .buffer_unordered(6)
                .collect()
                .await;
                changed = outcomes.into_iter().any(|hit| hit);
            }
            server::source::ArtistView::Library => {}
        }
        if changed {
            self.library.invalidate(api::Table::Tracks);
        }
        let (overrides, photos) = self.db.artist_images().await.map_err(Self::db_error)?;
        Ok(names
            .into_iter()
            .filter(|name| {
                let normalized = utils::artist::normalize_artist_key(name);
                overrides.contains_key(&normalized) || photos.contains_key(&normalized)
            })
            .collect())
    }

    pub async fn genres(&self) -> Result<Vec<String>, ApiError> {
        let config = self.current().await;
        self.db
            .genres(&config.active_source)
            .await
            .map_err(Self::db_error)
    }

    pub async fn recent_tracks(&self, page: api::Page) -> Result<api::TrackPage, ApiError> {
        let config = self.current().await;
        let keys = self
            .db
            .recently_played(&config.active_source, u32::MAX)
            .await
            .map_err(Self::db_error)?;
        let total = keys.len().min(u32::MAX as usize) as u32;
        let page_keys: Vec<String> = keys
            .into_iter()
            .skip(page.offset as usize)
            .take(page.limit as usize)
            .collect();
        let tracks = self
            .db
            .tracks_by_keys(&config.active_source, &page_keys)
            .await
            .map_err(Self::db_error)?;
        Ok(api::TrackPage {
            total,
            offset: page.offset,
            items: tracks
                .iter()
                .map(|track| crate::wire::track_info(track, &config))
                .collect(),
        })
    }

    pub async fn album_tracks(
        &self,
        id: &str,
        page: api::Page,
    ) -> Result<api::TrackPage, ApiError> {
        let config = self.current().await;
        let tracks = self
            .db
            .album_tracks(&config.active_source, id)
            .await
            .map_err(Self::db_error)?;
        Ok(Self::track_page(&tracks, page, &config))
    }

    pub async fn artist_tracks(
        &self,
        name: &str,
        page: api::Page,
    ) -> Result<api::TrackPage, ApiError> {
        let config = self.current().await;
        let tracks = self
            .db
            .artist_tracks(&config.active_source, name, None)
            .await
            .map_err(Self::db_error)?;
        Ok(Self::track_page(&tracks, page, &config))
    }

    pub async fn genre_tracks(
        &self,
        name: &str,
        page: api::Page,
    ) -> Result<api::TrackPage, ApiError> {
        let config = self.current().await;
        let tracks = self
            .db
            .genre_tracks(&config.active_source, name)
            .await
            .map_err(Self::db_error)?;
        Ok(Self::track_page(&tracks, page, &config))
    }

    pub async fn artist_sample_tracks(&self, page: api::Page) -> Result<api::TrackPage, ApiError> {
        let config = self.current().await;
        let tracks = self
            .db
            .artist_sample_tracks(&config.active_source, u32::MAX)
            .await
            .map_err(Self::db_error)?;
        Ok(Self::track_page(&tracks, page, &config))
    }

    pub async fn tracks_by_keys(&self, keys: &[String]) -> Result<Vec<api::TrackInfo>, ApiError> {
        let config = self.current().await;
        let tracks = self
            .db
            .tracks_by_keys(&config.active_source, keys)
            .await
            .map_err(Self::db_error)?;
        Ok(tracks
            .iter()
            .map(|track| crate::wire::track_info(track, &config))
            .collect())
    }

    pub async fn track_web_url(&self, key: &str) -> Result<Option<String>, ApiError> {
        let config = self.current().await;
        let track = self
            .db
            .tracks_by_keys(&config.active_source, &[key.to_string()])
            .await
            .map_err(Self::db_error)?
            .into_iter()
            .next()
            .ok_or_else(|| ApiError::not_found("track not found"))?;
        Ok(self.source().await.web_url(&track))
    }

    pub async fn album_web_url(&self, id: &str) -> Result<Option<String>, ApiError> {
        if id.trim().is_empty() {
            return Err(ApiError::invalid_input("album id is required"));
        }
        let service = self.current().await.server.map(|server| server.service);
        Ok(match service {
            Some(config::MusicService::Spotify) => {
                Some(format!("https://open.spotify.com/album/{id}"))
            }
            Some(config::MusicService::YtMusic) => {
                Some(format!("https://music.youtube.com/browse/{id}"))
            }
            _ => None,
        })
    }

    pub async fn top_genre(&self) -> Result<Option<String>, ApiError> {
        let config = self.current().await;
        self.db
            .top_genre(&config.active_source)
            .await
            .map_err(Self::db_error)
    }

    pub async fn search(&self, query: &str) -> Result<api::SearchResults, ApiError> {
        let config = self.current().await;
        let source: server::source::ActiveSource =
            Arc::from(server::source::active(self.db.clone(), &config));
        let (tracks, albums) = source.search(query).await.map_err(Self::source_error)?;
        self.library.register_transient(&tracks);
        Ok(api::SearchResults {
            tracks: tracks
                .iter()
                .map(|track| crate::wire::track_info(track, &config))
                .collect(),
            albums: albums.iter().map(Self::album_info).collect(),
        })
    }
}
