use async_trait::async_trait;
use config::{MusicService, Source};
use db::Db;

use crate::{server_ops::ServerConn, subsonic::SubsonicClient};

use super::{
    AlbumType, ArtistView, AuthOutcome, Capabilities, FavoritesSync, LibrarySnapshot, MediaSource,
    PlaylistMeta, PlaylistOps, SourceError, StreamInfo, encode_cover_url_tag, mirror_added,
    mirror_created,
};

pub(super) struct SubsonicSource {
    db: Db,
    source: Source,
    client: SubsonicClient,
    /// Subsonic or Custom — both use this impl; the typed track id needs the
    /// exact one so its uid prefix round-trips.
    service: MusicService,
}

impl SubsonicSource {
    pub(super) fn new(db: Db, source: Source, conn: &ServerConn) -> Self {
        Self {
            db,
            source,
            client: SubsonicClient::new(&conn.url, &conn.user_id, &conn.token),
            service: conn.service,
        }
    }
}

#[async_trait]
impl MediaSource for SubsonicSource {
    fn source(&self) -> &Source {
        &self.source
    }
    fn db(&self) -> &Db {
        &self.db
    }

    async fn scrobble_now_playing(&self, item_id: &str) -> Result<(), SourceError> {
        self.client
            .scrobble_now_playing(item_id)
            .await
            .map_err(SourceError::from)
    }

    async fn scrobble(&self, item_id: &str) -> Result<(), SourceError> {
        self.client
            .scrobble(item_id)
            .await
            .map_err(SourceError::from)
    }

    async fn fetch_library(&self) -> Result<LibrarySnapshot, SourceError> {
        use std::path::PathBuf;
        let prefix = match self.service {
            MusicService::Custom => "custom",
            _ => "subsonic",
        };
        let mut albums = Vec::new();
        let mut tracks = Vec::new();
        let mut artist_images = Vec::new();
        let mut seen = std::collections::HashSet::new();

        if let Ok(artists) = self.client.get_artists().await {
            for artist in artists {
                if let Some(cover_art_id) = &artist.cover_art
                    && let Ok(url) = self.client.cover_art_url(cover_art_id, Some(512))
                {
                    artist_images.push((artist.name, url));
                }
            }
        }

        let mut offset = 0;
        let batch = 250;
        loop {
            let page = self.client.get_album_list(offset, batch).await?;
            if page.is_empty() {
                break;
            }
            let count = page.len();
            for album in page {
                let album_cover_tag = album
                    .cover_art
                    .as_ref()
                    .and_then(|c| self.client.cover_art_url(c, Some(512)).ok())
                    .map(|url| encode_cover_url_tag(&url));
                let album_id_prefixed = match &album_cover_tag {
                    Some(tag) => format!("{}:{}:{}", prefix, album.id, tag),
                    None => format!("{}:{}:none", prefix, album.id),
                };
                let album_name = album.name.clone();
                let album_artist = album.artist.clone().unwrap_or_default();
                albums.push(reader::Album {
                    id: album_id_prefixed.clone(),
                    title: album_name.clone(),
                    artist: album_artist.clone(),
                    genre: album.genre.clone().unwrap_or_default(),
                    year: album.year.unwrap_or(0),
                    cover_path: Some(PathBuf::from(album_id_prefixed.clone())),
                    manual_cover: false,
                });

                let songs = self.client.get_album_songs(&album.id).await.map_err(|e| {
                    SourceError::Backend(format!(
                        "failed to fetch songs for album {}: {e}",
                        album.id
                    ))
                })?;
                for song in songs {
                    if !seen.insert(song.id.clone()) {
                        continue;
                    }
                    let bitrate_u16 = song.bit_rate.unwrap_or(0).min(u16::MAX as u32) as u16;
                    let song_cover_tag = song
                        .cover_art
                        .as_ref()
                        .and_then(|c| self.client.cover_art_url(c, Some(512)).ok())
                        .map(|url| encode_cover_url_tag(&url));
                    tracks.push(reader::Track {
                        id: reader::models::TrackId::Server {
                            service: self.service,
                            item_id: song.id.clone(),
                        },
                        cover: Some(song_cover_tag.unwrap_or_else(|| "none".to_string())),
                        album_id: album_id_prefixed.clone(),
                        title: song.title,
                        artist: song.artist.clone().unwrap_or_else(|| album_artist.clone()),
                        album: song.album.unwrap_or_else(|| album_name.clone()),
                        duration: song.duration.unwrap_or(0),
                        khz: song.sampling_rate.unwrap_or(0),
                        bitrate: bitrate_u16,
                        track_number: song.track,
                        disc_number: song.disc_number,
                        musicbrainz_release_id: None,
                        musicbrainz_recording_id: None,
                        musicbrainz_track_id: None,
                        playlist_item_id: None,
                        artists: vec![song.artist.unwrap_or_else(|| album_artist.clone())],
                    });
                }
            }
            offset += count;
            if count < batch {
                break;
            }
        }

        Ok(LibrarySnapshot {
            albums,
            tracks,
            artist_images,
        })
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
        position: usize,
    ) -> Result<(), SourceError> {
        self.client
            .remove_from_playlist(playlist_id, position)
            .await?;
        self.db
            .remove_playlist_tracks(&self.source, playlist_id, &[track.id.key().into_owned()])
            .await
            .map_err(SourceError::from)
    }

    async fn resolve_stream(&self, item_id: &str) -> Result<StreamInfo, SourceError> {
        Ok(StreamInfo {
            url: self.client.stream_url(item_id)?,
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
            Err(e)
                if e.contains("Wrong username")
                    || e.contains("not authorized")
                    || e.contains("code 40") =>
            {
                AuthOutcome::Expired
            }
            Err(_) => AuthOutcome::Unreachable,
        }
    }

    async fn fetch_favorites(&self) -> Result<Vec<String>, SourceError> {
        self.client
            .get_starred_song_ids()
            .await
            .map_err(SourceError::from)
    }

    async fn push_favorite(&self, item_id: &str, on: bool) -> Result<(), SourceError> {
        if on {
            self.client.star(item_id).await
        } else {
            self.client.unstar(item_id).await
        }
        .map_err(SourceError::from)
    }

    async fn reorder_playlist(
        &self,
        playlist_id: &str,
        ordered_refs: &[String],
        _moved: &reader::Track,
        _new_index: usize,
    ) -> Result<(), SourceError> {
        let ids: Vec<&str> = ordered_refs.iter().map(String::as_str).collect();
        self.client
            .reorder_playlist(playlist_id, &ids, ids.len())
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
                image_tag: None,
            })
            .collect())
    }

    async fn fetch_playlist_entries(
        &self,
        playlist_id: &str,
    ) -> Result<Vec<reader::Track>, SourceError> {
        let items = self.client.get_playlist_entries(playlist_id).await?;
        Ok(items
            .into_iter()
            .map(|item| {
                // Encode the cover URL as the `urlhex_` tag the cover resolver
                // understands; album_id carries it (or `:none`) like subsonic_sync.
                let cover_tag = item
                    .cover_art
                    .as_ref()
                    .and_then(|id| self.client.cover_art_url(id, Some(512)).ok())
                    .map(|url| format!("urlhex_{}", hex::encode(url.as_bytes())));
                let album_id = item
                    .album_id
                    .as_ref()
                    .map(|id| match &cover_tag {
                        Some(tag) => format!("jellyfin:{}:{}", id, tag),
                        None => format!("jellyfin:{}:none", id),
                    })
                    .unwrap_or_else(|| format!("jellyfin:{}:none", item.id));
                let artist = item.artist.clone().unwrap_or_default();
                reader::models::Track {
                    id: reader::models::TrackId::Server {
                        service: self.service,
                        item_id: item.id.clone(),
                    },
                    cover: Some(cover_tag.unwrap_or_else(|| "none".to_string())),
                    album_id,
                    title: item.title,
                    artist: artist.clone(),
                    album: item.album.unwrap_or_default(),
                    duration: item.duration.unwrap_or(0),
                    khz: item.sampling_rate.unwrap_or(0),
                    bitrate: item.bit_rate.unwrap_or(0).min(u16::MAX as u32) as u16,
                    track_number: item.track,
                    disc_number: item.disc_number,
                    musicbrainz_release_id: None,
                    musicbrainz_recording_id: None,
                    musicbrainz_track_id: None,
                    playlist_item_id: None,
                    artists: vec![artist],
                }
            })
            .collect())
    }

    async fn fetch_artist_images(&self) -> Result<Vec<(String, String)>, SourceError> {
        let artists = self.client.get_artists().await?;
        let mut out = Vec::new();
        for artist in artists {
            if let Some(cover_art_id) = &artist.cover_art
                && let Ok(url) = self.client.cover_art_url(cover_art_id, Some(512))
            {
                out.push((artist.name.clone(), url));
            }
        }
        Ok(out)
    }
}
