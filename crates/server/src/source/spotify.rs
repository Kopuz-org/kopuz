use std::sync::Arc;

use async_trait::async_trait;
use config::Source;
use db::Db;

use crate::server_ops::ServerConn;
use crate::spotify::catalog;
use crate::spotify::session::{SessionError, SpotifySession};

use super::{
    AlbumType, ArtistView, AuthOutcome, Capabilities, FavoritesPage, FavoritesSync,
    LibrarySnapshot, MediaSource, PlaylistMeta, PlaylistOps, RadioSeeds, SourceError, StreamInfo,
};

/// Spotify over a librespot session. Library, liked songs, playlists and
/// search go through the client's own endpoints on that session; audio is a
/// stream ref the decode worker opens through the same session
/// (`crate::spotify::stream`), so a track plays in the engine like any other
/// remote file.
pub(super) struct SpotifySource {
    db: Db,
    source: Source,
    session: Arc<SpotifySession>,
}

impl SpotifySource {
    pub(super) fn new(db: Db, source: Source, conn: &ServerConn) -> Self {
        let session = crate::spotify::session::shared(&conn.token, &conn.device_id);
        Self {
            db,
            source,
            session,
        }
    }

    /// The live session, connecting first if need be.
    async fn live(&self) -> Result<librespot_core::Session, SourceError> {
        self.session.session().await.map_err(|error| match error {
            SessionError::Unauthenticated(_) => SourceError::Auth,
            SessionError::Unavailable(message) => SourceError::Backend(message),
        })
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

    fn web_url(&self, track: &reader::Track) -> Option<String> {
        (track.id.service() == Some(config::MusicService::Spotify))
            .then(|| format!("https://open.spotify.com/track/{}", track.id.key()))
    }

    fn album_web_url(&self, browse_id: &str) -> Option<String> {
        (!browse_id.trim().is_empty())
            .then(|| format!("https://open.spotify.com/album/{browse_id}"))
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
            discover: true,
            dont_recommend: false,
            radio: RadioSeeds::NONE,
            playlists: PlaylistOps::None,
            artist_view: ArtistView::Library,
            albums: AlbumType::Standard,
            favorites_sync: FavoritesSync::Paginated,
        }
    }

    /// The stream is opened lazily on the decode worker, which needs a
    /// connected session: connecting here surfaces a refused login as a
    /// resolve error instead of a silent decoder failure.
    async fn resolve_stream(&self, item_id: &str) -> Result<StreamInfo, SourceError> {
        self.live().await?;
        Ok(StreamInfo {
            url: crate::playback_ref::ResolvedStreamRef::spotify_marker(item_id),
            format: None,
            user_agent: None,
            duration_secs: None,
            bitrate: None,
            content_length: None,
        })
    }

    async fn validate(&self) -> AuthOutcome {
        let session = match self.session.session().await {
            Ok(session) => session,
            Err(SessionError::Unauthenticated(_)) => return AuthOutcome::Expired,
            Err(SessionError::Unavailable(_)) => return AuthOutcome::Unreachable,
        };
        match catalog::probe(&session).await {
            Ok(()) => AuthOutcome::Valid,
            Err(e) if e.contains("401") || e.contains("403") => AuthOutcome::Expired,
            Err(_) => AuthOutcome::Unreachable,
        }
    }

    async fn fetch_favorites(&self) -> Result<Vec<String>, SourceError> {
        let session = self.live().await?;
        Ok(catalog::collection(&session)
            .await?
            .into_iter()
            .filter(|item| item.kind == catalog::CollectionKind::Track)
            .filter_map(|item| item.id.to_base62().ok())
            .collect())
    }

    /// Liked songs, newest first. The collection is one list, so the
    /// cursor is an offset into it and each page hydrates its own ids.
    async fn fetch_favorites_page(
        &self,
        cursor: Option<String>,
    ) -> Result<FavoritesPage, SourceError> {
        const PAGE: usize = 50;
        let session = self.live().await?;
        let offset: usize = cursor.and_then(|c| c.parse().ok()).unwrap_or(0);
        let liked: Vec<String> = catalog::collection(&session)
            .await?
            .into_iter()
            .filter(|item| item.kind == catalog::CollectionKind::Track)
            .filter_map(|item| item.id.to_base62().ok())
            .collect();
        let page: Vec<librespot_core::SpotifyUri> = liked
            .iter()
            .skip(offset)
            .take(PAGE)
            .filter_map(|id| {
                librespot_core::SpotifyUri::from_uri(&format!("spotify:track:{id}")).ok()
            })
            .collect();
        let tracks = catalog::tracks(&session, &page).await?;
        let next = (offset + PAGE < liked.len()).then(|| (offset + PAGE).to_string());
        Ok(FavoritesPage { tracks, next })
    }

    async fn push_favorite(&self, item_id: &str, on: bool) -> Result<(), SourceError> {
        let session = self.live().await?;
        catalog::set_liked(&session, item_id, on)
            .await
            .map_err(SourceError::from)
    }

    async fn search(
        &self,
        query: &str,
    ) -> Result<(Vec<reader::Track>, Vec<reader::Album>), SourceError> {
        let session = self.live().await?;
        crate::spotify::search::search(&session, query)
            .await
            .map_err(SourceError::from)
    }

    async fn fetch_album_tracks(&self, album_id: &str) -> Result<Vec<reader::Track>, SourceError> {
        let session = self.live().await?;
        catalog::album(&session, album_id)
            .await
            .map(|album| album.tracks)
            .map_err(SourceError::from)
    }

    async fn fetch_album_by_ref(
        &self,
        id: &str,
    ) -> Result<Option<super::RemoteAlbum>, SourceError> {
        let session = self.live().await?;
        catalog::album(&session, id)
            .await
            .map(|a| (!a.tracks.is_empty()).then_some(a))
            .map_err(SourceError::from)
    }

    async fn discover_home(&self) -> Result<crate::ytmusic::discover::DiscoverHome, SourceError> {
        let session = self.live().await?;
        catalog::discover_home(&session)
            .await
            .map_err(SourceError::from)
    }

    async fn fetch_library(&self) -> Result<LibrarySnapshot, SourceError> {
        let session = self.live().await?;
        let (albums, tracks) = catalog::saved_albums(&session).await?;
        Ok(LibrarySnapshot {
            albums,
            tracks,
            artist_images: Vec::new(),
        })
    }

    async fn fetch_playlists(&self) -> Result<Vec<PlaylistMeta>, SourceError> {
        let session = self.live().await?;
        Ok(catalog::playlists(&session)
            .await?
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
        let session = self.live().await?;
        catalog::playlist_tracks(&session, playlist_id)
            .await
            .map_err(SourceError::from)
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
