use async_trait::async_trait;
use config::{MusicService, Source};
use db::Db;

use crate::{jellyfin::JellyfinClient, server_ops::ServerConn};

use super::{
    AlbumType, ArtistView, AuthOutcome, Capabilities, FavoritesSync, LibrarySnapshot, MediaSource,
    PlaylistMeta, PlaylistOps, SourceError, StreamInfo, mirror_added, mirror_created,
};

pub(super) struct JellyfinSource {
    db: Db,
    source: Source,
    client: JellyfinClient,
}

impl JellyfinSource {
    pub(super) fn new(db: Db, source: Source, conn: &ServerConn) -> Self {
        Self {
            db,
            source,
            client: JellyfinClient::new(
                &conn.url,
                Some(&conn.token),
                &conn.device_id,
                Some(&conn.user_id),
            ),
        }
    }
}

#[async_trait]
impl MediaSource for JellyfinSource {
    fn source(&self) -> &Source {
        &self.source
    }
    fn db(&self) -> &Db {
        &self.db
    }

    async fn keepalive(&self) -> Result<(), SourceError> {
        self.client.ping().await.map_err(SourceError::from)
    }

    async fn report_playback_start(&self, item_id: &str) -> Result<(), SourceError> {
        self.client
            .report_playback_start(item_id)
            .await
            .map_err(SourceError::from)
    }

    async fn report_playback_stopped(
        &self,
        item_id: &str,
        position_ticks: u64,
    ) -> Result<(), SourceError> {
        self.client
            .report_playback_stopped(item_id, position_ticks)
            .await
            .map_err(SourceError::from)
    }

    async fn report_playback_progress(
        &self,
        item_id: &str,
        position_ticks: u64,
        is_paused: bool,
    ) -> Result<(), SourceError> {
        self.client
            .report_playback_progress(item_id, position_ticks, is_paused)
            .await
            .map_err(SourceError::from)
    }

    async fn fetch_library(&self) -> Result<LibrarySnapshot, SourceError> {
        use std::path::PathBuf;
        let mut albums = Vec::new();
        let mut tracks = Vec::new();
        let mut artist_images = Vec::new();

        for lib in self.client.get_music_libraries().await? {
            let mut start = 0;
            let limit = 500;
            loop {
                let (page, _total) = self
                    .client
                    .get_albums_paginated(&lib.id, start, limit)
                    .await?;
                if page.is_empty() {
                    break;
                }
                let count = page.len();
                for a in page {
                    let image_tag = a
                        .image_tags
                        .as_ref()
                        .and_then(|t| t.get("Primary").cloned());
                    let cover_path = Some(PathBuf::from(match &image_tag {
                        Some(tag) => format!("jellyfin:{}:{}", a.id, tag),
                        None => format!("jellyfin:{}", a.id),
                    }));
                    albums.push(reader::Album {
                        id: format!("jellyfin:{}", a.id),
                        title: a.name,
                        artist: a
                            .album_artist
                            .or_else(|| a.artists.as_ref().map(|x| x.join(", ")))
                            .unwrap_or_default(),
                        genre: a.genres.as_ref().map(|g| g.join(", ")).unwrap_or_default(),
                        year: a
                            .production_year
                            .map(|y| u16::try_from(y).unwrap_or(u16::MAX))
                            .unwrap_or(0),
                        cover_path,
                        manual_cover: false,
                    });
                }
                start += count;
                if count < limit {
                    break;
                }
            }

            let mut start = 0;
            let limit = 500;
            loop {
                let items = self
                    .client
                    .get_music_library_items_paginated(&lib.id, start, limit)
                    .await?;
                if items.is_empty() {
                    break;
                }
                let count = items.len();
                for item in items {
                    let cover = item
                        .image_tags
                        .as_ref()
                        .and_then(|tags| tags.get("Primary").cloned());
                    let bitrate_u16 =
                        (item.bitrate.unwrap_or(0) / 1000).min(u16::MAX as u32) as u16;
                    tracks.push(reader::Track {
                        id: reader::models::TrackId::Server {
                            service: MusicService::Jellyfin,
                            item_id: item.id.clone(),
                        },
                        cover,
                        album_id: item
                            .album_id
                            .map(|id| format!("jellyfin:{}", id))
                            .unwrap_or_default(),
                        title: item.name,
                        artist: item
                            .album_artist
                            .clone()
                            .or_else(|| item.artists.as_ref().map(|a| a.join(", ")))
                            .unwrap_or_default(),
                        album: item.album.unwrap_or_default(),
                        duration: item.run_time_ticks.unwrap_or(0) / 10_000_000,
                        khz: item.sample_rate.unwrap_or(0),
                        bitrate: bitrate_u16,
                        track_number: item.index_number,
                        disc_number: item.parent_index_number,
                        musicbrainz_release_id: None,
                        musicbrainz_recording_id: None,
                        musicbrainz_track_id: None,
                        playlist_item_id: None,
                        artists: item
                            .artists
                            .unwrap_or_else(|| item.album_artist.into_iter().collect()),
                    });
                }
                start += count;
                if count < limit {
                    break;
                }
            }
        }

        if let Ok(artists) = self.client.get_artists().await {
            for artist in artists {
                if let Some(tags) = &artist.image_tags
                    && let Some(tag) = tags.get("Primary")
                {
                    let url = utils::jellyfin_image::jellyfin_image_url(
                        self.client.base_url(),
                        &artist.id,
                        Some(tag.as_str()),
                        self.client.token(),
                        512,
                        90,
                    );
                    artist_images.push((artist.name, url));
                }
            }
        }

        Ok(LibrarySnapshot {
            albums,
            tracks,
            artist_images,
        })
    }

    async fn set_playlist_cover(
        &self,
        playlist_id: &str,
        name: &str,
        image_path: &std::path::Path,
        image_tag: Option<&str>,
    ) -> Result<(), SourceError> {
        // Push the artwork to Jellyfin (best-effort), then record it locally.
        if let Ok(bytes) = std::fs::read(image_path) {
            let ct = match image_path
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_lowercase)
                .as_deref()
            {
                Some("png") => "image/png",
                _ => "image/jpeg",
            };
            let _ = self.client.set_playlist_image(playlist_id, bytes, ct).await;
        }
        let cover = image_path.to_string_lossy();
        self.db()
            .upsert_playlist_meta(self.source(), playlist_id, name, Some(&cover), image_tag)
            .await
            .map_err(SourceError::from)
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
            radio: false,
            playlists: PlaylistOps::Reorder,
            artist_view: ArtistView::Library,
            albums: AlbumType::Standard,
            favorites_sync: FavoritesSync::Instant,
        }
    }

    async fn add_to_playlist(
        &self,
        playlist_id: &str,
        item_refs: &[String],
    ) -> Result<Vec<String>, SourceError> {
        let mut added = Vec::new();
        for id in item_refs {
            if self.client.add_to_playlist(playlist_id, id).await.is_ok() {
                added.push(id.clone());
            }
        }
        mirror_added(&self.db, &self.source, playlist_id, &added).await?;
        Ok(added)
    }

    async fn create_playlist(
        &self,
        name: &str,
        item_refs: &[String],
    ) -> Result<String, SourceError> {
        let refs: Vec<&str> = item_refs.iter().map(String::as_str).collect();
        let id = self.client.create_playlist(name, &refs).await?;
        mirror_created(&self.db, &self.source, &id, name, item_refs).await?;
        Ok(id)
    }

    async fn remove_from_playlist(
        &self,
        playlist_id: &str,
        track: &reader::Track,
        _position: usize,
    ) -> Result<(), SourceError> {
        let entry_id = track
            .playlist_item_id
            .as_deref()
            .ok_or_else(|| SourceError::InvalidInput("track has no playlist-entry id".into()))?;
        self.client
            .remove_from_playlist(playlist_id, entry_id)
            .await?;
        self.db
            .remove_playlist_tracks(&self.source, playlist_id, &[track.id.key().into_owned()])
            .await
            .map_err(SourceError::from)
    }

    async fn resolve_stream(&self, item_id: &str) -> Result<StreamInfo, SourceError> {
        Ok(StreamInfo {
            url: self.client.stream_url(item_id),
            format: None,
            user_agent: None,
            duration_secs: None,
            bitrate: None,
            content_length: None,
        })
    }

    async fn validate(&self) -> AuthOutcome {
        match self.client.ping().await {
            Ok(()) => AuthOutcome::Valid,
            Err(e) if e.contains("401") || e.contains("403") => AuthOutcome::Expired,
            Err(_) => AuthOutcome::Unreachable,
        }
    }

    async fn fetch_favorites(&self) -> Result<Vec<String>, SourceError> {
        Ok(self
            .client
            .get_favorite_items()
            .await?
            .into_iter()
            .map(|i| i.id)
            .collect())
    }

    async fn push_favorite(&self, item_id: &str, on: bool) -> Result<(), SourceError> {
        if on {
            self.client.mark_favorite(item_id).await
        } else {
            self.client.unmark_favorite(item_id).await
        }
        .map_err(SourceError::from)
    }

    async fn reorder_playlist(
        &self,
        playlist_id: &str,
        ordered_refs: &[String],
        moved: &reader::Track,
        new_index: usize,
    ) -> Result<(), SourceError> {
        let entry_id = moved
            .playlist_item_id
            .as_deref()
            .ok_or_else(|| SourceError::InvalidInput("track has no playlist-entry id".into()))?;
        self.client
            .move_playlist_item(playlist_id, entry_id, new_index)
            .await?;
        self.db
            .set_playlist_tracks(&self.source, playlist_id, ordered_refs)
            .await
            .map_err(SourceError::from)
    }

    async fn fetch_playlists(&self) -> Result<Vec<PlaylistMeta>, SourceError> {
        Ok(self
            .client
            .get_playlists()
            .await?
            .into_iter()
            .map(|p| PlaylistMeta {
                id: p.id,
                name: p.name,
                image_tag: p
                    .image_tags
                    .as_ref()
                    .and_then(|tags| tags.get("Primary").cloned()),
            })
            .collect())
    }

    async fn fetch_playlist_entries(
        &self,
        playlist_id: &str,
    ) -> Result<Vec<reader::Track>, SourceError> {
        let items = self.client.get_playlist_items(playlist_id).await?;
        Ok(items
            .into_iter()
            .map(|item| {
                let duration_secs = item.run_time_ticks.unwrap_or(0) / 10_000_000;
                let cover = item
                    .image_tags
                    .as_ref()
                    .and_then(|tags| tags.get("Primary").cloned());
                let bitrate_kbps = item.bitrate.unwrap_or(0) / 1000;
                let artist_str = item
                    .album_artist
                    .clone()
                    .or_else(|| item.artists.as_ref().map(|a| a.join(", ")))
                    .unwrap_or_default();
                reader::models::Track {
                    id: reader::models::TrackId::Server {
                        service: MusicService::Jellyfin,
                        item_id: item.id.clone(),
                    },
                    cover,
                    album_id: item
                        .album_id
                        .map(|id| format!("jellyfin:{}", id))
                        .unwrap_or_default(),
                    title: item.name,
                    artist: artist_str,
                    album: item.album.unwrap_or_default(),
                    duration: duration_secs,
                    khz: item.sample_rate.unwrap_or(0),
                    bitrate: bitrate_kbps.min(u16::MAX as u32) as u16,
                    track_number: item.index_number,
                    disc_number: item.parent_index_number,
                    musicbrainz_release_id: None,
                    musicbrainz_recording_id: None,
                    musicbrainz_track_id: None,
                    playlist_item_id: item.playlist_item_id,
                    artists: item.artists.unwrap_or_default(),
                }
            })
            .collect())
    }

    async fn fetch_artist_images(&self) -> Result<Vec<(String, String)>, SourceError> {
        let artists = self.client.get_artists().await?;
        let mut out = Vec::new();
        for artist in artists {
            if let Some(tags) = &artist.image_tags
                && let Some(tag) = tags.get("Primary")
            {
                out.push((
                    artist.name.clone(),
                    utils::jellyfin_image::jellyfin_image_url(
                        self.client.base_url(),
                        &artist.id,
                        Some(tag.as_str()),
                        self.client.token(),
                        512,
                        90,
                    ),
                ));
            }
        }
        Ok(out)
    }
}
