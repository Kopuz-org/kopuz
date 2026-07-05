use async_trait::async_trait;
use config::Source;
use db::Db;

use crate::server_ops::ServerConn;

use super::{
    AlbumType, ArtistView, AuthOutcome, Capabilities, FavoritesPage, FavoritesSync,
    LibrarySnapshot, MediaSource, PlaylistMeta, PlaylistOps, SourceError, StreamInfo,
};

/// librespot-backed Spotify source. Read-only in v1: it hydrates the user's
/// playlists and plays tracks (Premium). All network goes through Spotify's
/// internal access-point protocol (see [`crate::spotify`]), so library sync
/// doesn't hit the public Web API rate limits. `access_token` is the unpacked
/// OAuth access token; `None` means no usable creds.
pub(super) struct SpotifySource {
    db: Db,
    source: Source,
    access_token: Option<String>,
}

impl SpotifySource {
    pub(super) fn new(db: Db, source: Source, conn: &ServerConn) -> Self {
        Self {
            db,
            source,
            access_token: (!conn.token.is_empty())
                .then(|| crate::spotify::auth::unpack_token(&conn.token).0),
        }
    }
}

#[async_trait]
impl MediaSource for SpotifySource {
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
            downloads: false,
            discover: false,
            radio: false,
            playlists: PlaylistOps::None,
            artist_view: ArtistView::Library,
            albums: AlbumType::Standard,
            favorites_sync: FavoritesSync::Paginated,
        }
    }

    async fn resolve_stream(&self, item_id: &str) -> Result<StreamInfo, SourceError> {
        // Spotify is the catalog; the audio comes from an anonymous YouTube
        // match (full track, no Premium audio key needed). Reuses the YouTube
        // decode path via the resolved googlevideo stream.
        let token = self
            .access_token
            .as_deref()
            .ok_or_else(|| SourceError::Backend("spotify: no access token".into()))?;
        tracing::info!(item_id, "spotify: resolve_stream -> youtube match");
        // Reuse the track's already-synced DB row for the search query so we skip
        // a redundant Spotify metadata fetch on a cold play.
        let known = self
            .db
            .tracks_by_keys(&self.source, &[item_id.to_string()])
            .await
            .ok()
            .and_then(|mut v| v.pop());
        let info = crate::spotify::match_yt::resolve(token, item_id, known)
            .await
            .map_err(SourceError::Backend)?;
        Ok(StreamInfo {
            url: info.url,
            format: Some((info.format, info.range_safe)),
            user_agent: Some(info.user_agent),
            duration_secs: info.duration_secs,
            bitrate: info.bitrate,
            content_length: info.content_length,
        })
    }

    async fn validate(&self) -> AuthOutcome {
        match self.access_token.as_deref() {
            None => AuthOutcome::Expired,
            Some(token) => match crate::spotify::auth::validate(token).await {
                Ok(()) => AuthOutcome::Valid,
                // Can't cleanly separate an expired token from a network blip,
                // so don't force re-sign-in: treat any failure as unreachable.
                Err(_) => AuthOutcome::Unreachable,
            },
        }
    }

    async fn fetch_library(&self) -> Result<LibrarySnapshot, SourceError> {
        let Some(token) = self.access_token.clone() else {
            return Ok(LibrarySnapshot::default());
        };
        let tracks = crate::spotify::metadata::liked_tracks(token)
            .await
            .map_err(SourceError::Backend)?;

        let mut seen = std::collections::HashSet::new();
        let mut albums = Vec::new();
        for t in &tracks {
            if !t.album_id.is_empty() && seen.insert(t.album_id.clone()) {
                albums.push(reader::Album {
                    id: t.album_id.clone(),
                    title: t.album.clone(),
                    artist: t.artist.clone(),
                    genre: String::new(),
                    year: 0,
                    cover_path: None,
                    manual_cover: false,
                });
            }
        }
        Ok(LibrarySnapshot {
            albums,
            tracks,
            artist_images: Vec::new(),
        })
    }

    async fn fetch_favorites(&self) -> Result<Vec<String>, SourceError> {
        let Some(token) = self.access_token.clone() else {
            return Ok(Vec::new());
        };
        Ok(crate::spotify::metadata::liked_tracks(token)
            .await
            .map_err(SourceError::Backend)?
            .into_iter()
            .map(|t| t.id.key().into_owned())
            .collect())
    }

    async fn fetch_favorites_page(
        &self,
        cursor: Option<String>,
    ) -> Result<FavoritesPage, SourceError> {
        if cursor.is_some() {
            return Ok(FavoritesPage {
                tracks: Vec::new(),
                next: None,
            });
        }
        let Some(token) = self.access_token.clone() else {
            return Ok(FavoritesPage {
                tracks: Vec::new(),
                next: None,
            });
        };
        let tracks = crate::spotify::metadata::liked_tracks(token)
            .await
            .map_err(SourceError::Backend)?;
        Ok(FavoritesPage { tracks, next: None })
    }

    async fn push_favorite(&self, _item_id: &str, _on: bool) -> Result<(), SourceError> {
        Err(SourceError::unsupported("favorite"))
    }

    async fn fetch_playlists(&self) -> Result<Vec<PlaylistMeta>, SourceError> {
        let Some(token) = self.access_token.clone() else {
            tracing::warn!("spotify: fetch_playlists with no access token");
            return Ok(Vec::new());
        };
        tracing::info!("spotify: fetch_playlists start");
        Ok(crate::spotify::metadata::list_playlists(token)
            .await
            .map_err(SourceError::Backend)?
            .into_iter()
            .map(|p| PlaylistMeta {
                id: p.id,
                name: p.name,
                image_tag: p.image,
            })
            .collect())
    }

    async fn fetch_playlist_entries(
        &self,
        playlist_id: &str,
    ) -> Result<Vec<reader::Track>, SourceError> {
        let Some(token) = self.access_token.clone() else {
            return Ok(Vec::new());
        };
        crate::spotify::metadata::playlist_entries(token, playlist_id.to_string())
            .await
            .map_err(SourceError::Backend)
    }

    async fn add_to_playlist(
        &self,
        _playlist_id: &str,
        _item_refs: &[String],
    ) -> Result<Vec<String>, SourceError> {
        Err(SourceError::unsupported("playlist add"))
    }

    async fn create_playlist(
        &self,
        _name: &str,
        _item_refs: &[String],
    ) -> Result<String, SourceError> {
        Err(SourceError::unsupported("playlist create"))
    }

    async fn remove_from_playlist(
        &self,
        _playlist_id: &str,
        _track: &reader::Track,
        _position: usize,
    ) -> Result<(), SourceError> {
        Err(SourceError::unsupported("playlist remove"))
    }
}
