//! Apple Music implementation of the unified media-source facade.

use async_trait::async_trait;
use config::Source;
use db::Db;

use super::{
    AlbumType, ArtistView, AuthOutcome, Capabilities, FavoritesSync, LibrarySnapshot, MediaSource,
    PlaylistMeta, PlaylistOps, RadioSeeds, SourceError, StreamInfo,
};

pub(super) struct AppleMusicSource {
    db: Db,
    source: Source,
    client: crate::applemusic::AppleMusicApi,
}

impl AppleMusicSource {
    pub(super) fn new(db: Db, source: Source, client: crate::applemusic::AppleMusicApi) -> Self {
        Self { db, source, client }
    }
}

#[async_trait]
impl MediaSource for AppleMusicSource {
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
            sync: true,
            downloads: true,
            discover: false,
            radio: RadioSeeds::ALL,
            playlists: PlaylistOps::AddRemove,
            artist_view: ArtistView::Library,
            albums: AlbumType::Standard,
            favorites_sync: FavoritesSync::Instant,
        }
    }

    /// Apple's stations are song-seeded, so the seed has to be a song Apple
    /// sells: an uploaded library track has no catalog entry and therefore no
    /// station. `resolve_catalog_id` hands back the library id unchanged in
    /// that case, which is what the numeric check catches.
    async fn start_radio(&self, seed_ref: &str) -> Result<Vec<reader::Track>, SourceError> {
        const QUEUE_TARGET: usize = 30;

        if seed_ref.trim().is_empty() {
            return Err(SourceError::InvalidInput("track has no id".into()));
        }
        let catalog_id = self
            .client
            .resolve_catalog_id(seed_ref)
            .await
            .map_err(SourceError::Backend)?;
        if !catalog_id.chars().all(|c| c.is_ascii_digit()) {
            return Err(SourceError::InvalidInput(
                "this track isn't in the Apple Music catalog, so it can't seed a station".into(),
            ));
        }

        let station = self
            .client
            .song_station_id(&catalog_id)
            .await
            .map_err(SourceError::Backend)?
            .ok_or_else(|| SourceError::Backend("this track has no station".to_string()))?;

        let songs = self
            .client
            .station_queue(&station, QUEUE_TARGET)
            .await
            .map_err(SourceError::Backend)?;
        Ok(songs
            .iter()
            .map(crate::applemusic::track_from_song_data)
            .collect())
    }

    /// The playlist equivalent of [`start_radio`], and what Apple's own client
    /// calls autoplay: a station built from what the playlist contains, rather
    /// than from one song.
    async fn start_playlist_radio(
        &self,
        playlist_ref: &str,
    ) -> Result<Vec<reader::Track>, SourceError> {
        const QUEUE_TARGET: usize = 30;
        /// Apple's own client sends ten. More than one matters: a playlist that
        /// opens with uploads would be refused on those alone, while a later
        /// catalog track still gives it something to work from.
        const SEEDS: usize = 10;

        if playlist_ref.trim().is_empty() {
            return Err(SourceError::InvalidInput("playlist has no id".into()));
        }

        let tracks = self
            .client
            .get_library_playlist_tracks(playlist_ref)
            .await
            .map_err(SourceError::Backend)?;
        // The library row id, not the catalog id: the seeds name rows of *this*
        // playlist, which is what ties them to the container.
        let seeds: Vec<String> = tracks.iter().take(SEEDS).map(|t| t.id.clone()).collect();
        if seeds.is_empty() {
            return Err(SourceError::InvalidInput(
                "an empty playlist can't start a station".into(),
            ));
        }

        let station = self
            .client
            .playlist_station_id(playlist_ref, &seeds)
            .await
            .map_err(SourceError::Backend)?;

        let songs = self
            .client
            .station_queue(&station, QUEUE_TARGET)
            .await
            .map_err(SourceError::Backend)?;
        Ok(songs
            .iter()
            .map(crate::applemusic::track_from_song_data)
            .collect())
    }

    async fn download_track(
        &self,
        item_id: &str,
        progress: Option<utils::stream_buffer::BufferProgressCallback>,
    ) -> Result<Vec<u8>, SourceError> {
        let token = self
            .client
            .media_user_token()
            .ok_or_else(|| SourceError::InvalidInput("no Apple Music user token".into()))?;
        crate::applemusic::stream::download_decrypted(
            item_id,
            token,
            self.client.storefront(),
            self.client.language(),
            progress,
        )
        .await
        .map_err(SourceError::Backend)
    }

    async fn resolve_stream(&self, _item_id: &str) -> Result<StreamInfo, SourceError> {
        let token = self.client.media_user_token().unwrap_or("");
        let encoded_token =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, token.as_bytes());
        // Parsed by `ResolvedStreamRef::apple_music_parts` on the hooks side.
        Ok(StreamInfo {
            url: format!(
                "__AM_FMP4:{_item_id}:{}:{}:{encoded_token}",
                self.client.storefront(),
                self.client.language()
            ),
            format: None,
            user_agent: None,
            duration_secs: None,
            bitrate: Some(256_000),
            content_length: None,
        })
    }

    async fn validate(&self) -> AuthOutcome {
        match self.client.validate().await {
            Ok(()) => AuthOutcome::Valid,
            Err(e) => {
                tracing::warn!("am.source.validate: {e}");
                if e.contains("expired") || e.contains("401") {
                    AuthOutcome::Expired
                } else {
                    AuthOutcome::Unreachable
                }
            }
        }
    }

    async fn search(
        &self,
        query: &str,
    ) -> Result<(Vec<reader::Track>, Vec<reader::Album>), SourceError> {
        if query.trim().is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }
        let resp = self
            .client
            .search(query, "songs,albums", 25, 0)
            .await
            .map_err(SourceError::Backend)?;

        let tracks: Vec<reader::Track> = resp
            .results
            .songs
            .map(|s| {
                s.data
                    .iter()
                    .map(crate::applemusic::track_from_song_data)
                    .collect()
            })
            .unwrap_or_default();

        let albums: Vec<reader::Album> = resp
            .results
            .albums
            .map(|a| {
                a.data
                    .iter()
                    .map(|a| reader::Album {
                        id: format!("applemusic:{}", a.id),
                        title: a.attributes.name.clone(),
                        artist: a.attributes.artist_name.clone(),
                        genre: a.attributes.genreNames.join(", "),
                        year: a
                            .attributes
                            .releaseDate
                            .split('-')
                            .next()
                            .and_then(|y| y.parse().ok())
                            .unwrap_or(0),
                        cover_path: Some(std::path::PathBuf::from(format!(
                            "applemusic:{}:{}",
                            a.id,
                            crate::applemusic::artwork_url(&a.attributes.artwork.url, 600)
                        ))),
                        manual_cover: false,
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok((tracks, albums))
    }

    async fn fetch_library(&self) -> Result<LibrarySnapshot, SourceError> {
        let mut albums = Vec::new();
        let mut tracks = Vec::new();

        let library_albums = self
            .client
            .get_library_albums()
            .await
            .map_err(SourceError::Backend)?;
        for la in &library_albums {
            albums.push(crate::applemusic::album_from_library(la));
        }

        let library_songs = self
            .client
            .get_library_songs()
            .await
            .map_err(SourceError::Backend)?;
        for ls in &library_songs {
            tracks.push(crate::applemusic::track_from_library_song(ls));
        }

        Ok(LibrarySnapshot {
            albums,
            tracks,
            artist_images: Vec::new(),
        })
    }

    async fn fetch_favorites(&self) -> Result<Vec<String>, SourceError> {
        let ids = self
            .client
            .get_favorites()
            .await
            .map_err(SourceError::Backend)?;
        Ok(ids
            .into_iter()
            .map(|id| crate::applemusic::apple_music_id(&id).key().into_owned())
            .collect())
    }

    async fn push_favorite(&self, item_id: &str, on: bool) -> Result<(), SourceError> {
        // The heart, not the library. `fetch_favorites` reads the Favorite Songs
        // playlist, which is what this endpoint feeds; adding to the library
        // instead left the two halves describing different sets.
        self.client
            .set_favorite(item_id, on)
            .await
            .map_err(SourceError::Backend)
    }

    async fn fetch_playlists(&self) -> Result<Vec<PlaylistMeta>, SourceError> {
        let playlists = self
            .client
            .get_library_playlists()
            .await
            .map_err(SourceError::Backend)?;
        Ok(playlists
            .into_iter()
            .map(|p| PlaylistMeta {
                id: p.id,
                name: p.attributes.name,
                image_tag: p.attributes.artwork.map(|a| {
                    reader::CoverRef::encode_url(&crate::applemusic::artwork_url(&a.url, 300))
                }),
            })
            .collect())
    }

    async fn fetch_playlist_entries(
        &self,
        playlist_id: &str,
    ) -> Result<Vec<reader::Track>, SourceError> {
        let songs = self
            .client
            .get_library_playlist_tracks(playlist_id)
            .await
            .map_err(SourceError::Backend)?;
        Ok(songs
            .iter()
            .map(crate::applemusic::track_from_playlist_entry)
            .collect())
    }

    async fn add_to_playlist(
        &self,
        playlist_id: &str,
        item_refs: &[String],
    ) -> Result<Vec<String>, SourceError> {
        self.client
            .add_to_playlist(playlist_id, item_refs)
            .await
            .map_err(SourceError::Backend)?;
        Ok(item_refs.to_vec())
    }

    async fn create_playlist(
        &self,
        name: &str,
        item_refs: &[String],
    ) -> Result<String, SourceError> {
        self.client
            .create_playlist(name, item_refs)
            .await
            .map_err(SourceError::Backend)
    }

    async fn remove_from_playlist(
        &self,
        playlist_id: &str,
        track: &reader::Track,
        _position: usize,
    ) -> Result<(), SourceError> {
        // The playlist row's own id, not the catalog id — see
        // `track_from_playlist_entry`.
        let entry_id = track
            .playlist_item_id
            .as_deref()
            .ok_or_else(|| SourceError::InvalidInput("track has no playlist-entry id".into()))?;
        self.client
            .remove_from_playlist(playlist_id, entry_id)
            .await
            .map_err(SourceError::Backend)?;
        self.db
            .remove_playlist_tracks(&self.source, playlist_id, &[track.id.key().into_owned()])
            .await
            .map_err(SourceError::from)
    }

    async fn fetch_artist_images(&self) -> Result<Vec<(String, String)>, SourceError> {
        tracing::info!("am.fetch_artist_images: starting");
        let artists = self
            .client
            .get_library_artists()
            .await
            .map_err(SourceError::Backend)?;
        let mut out = Vec::new();
        for a in &artists {
            if let Some(artwork) = &a.attributes.artwork
                && !artwork.url.is_empty()
            {
                let url = crate::applemusic::artwork_url(&artwork.url, 300);
                out.push((a.attributes.name.clone(), url));
            }
        }
        tracing::info!("am.fetch_artist_images: {} artists with images", out.len());
        Ok(out)
    }
}
