use async_trait::async_trait;
use config::Source;
use db::Db;

use crate::server_ops::ServerConn;

use super::{
    AlbumType, ArtistView, AuthOutcome, Capabilities, FavoritesPage, FavoritesSync,
    LibrarySnapshot, MediaSource, PlaylistMeta, PlaylistOps, SourceError, StreamInfo,
};

/// Read-only Spotify Web API source. Playback does NOT flow through this impl:
/// the player controller intercepts Spotify tracks and drives the Web Playback
/// SDK in the user's browser (`crate::spotify::host`), so `resolve_stream` is
/// never reached on the happy path. Everything else — library, liked songs,
/// playlists, search — comes from the public Web API.
pub(super) struct SpotifySource {
    db: Db,
    source: Source,
    /// The Web API access token, unpacked from the stored `<access>\n<refresh>`.
    access: String,
}

impl SpotifySource {
    pub(super) fn new(db: Db, source: Source, conn: &ServerConn) -> Self {
        let access = crate::spotify::auth::unpack_token(&conn.token).0;
        Self { db, source, access }
    }

    fn token(&self) -> Result<&str, SourceError> {
        if self.access.is_empty() {
            Err(SourceError::Auth)
        } else {
            Ok(self.access.as_str())
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
            // Read-only for now (playlist mutation not wired).
            playlists: PlaylistOps::None,
            artist_view: ArtistView::Library,
            albums: AlbumType::Standard,
            favorites_sync: FavoritesSync::Paginated,
        }
    }

    async fn resolve_stream(&self, _item_id: &str) -> Result<StreamInfo, SourceError> {
        // Spotify audio is DRM'd and never decoded by the engine; the controller
        // routes these tracks to the browser SDK before reaching here.
        Err(SourceError::unsupported(
            "Spotify playback (handled by the browser player)",
        ))
    }

    async fn validate(&self) -> AuthOutcome {
        let Ok(token) = self.token() else {
            return AuthOutcome::Expired;
        };
        match crate::spotify::api::me(token).await {
            Ok(()) => AuthOutcome::Valid,
            // Can't cleanly separate an expired token from a network blip here,
            // so treat any failure as unreachable rather than forcing re-signin.
            Err(_) => AuthOutcome::Unreachable,
        }
    }

    async fn fetch_favorites(&self) -> Result<Vec<String>, SourceError> {
        let token = self.token()?;
        let mut ids = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let (tracks, next) =
                crate::spotify::api::saved_tracks_page(token, cursor.as_deref()).await?;
            ids.extend(tracks.iter().map(|t| t.id.key().into_owned()));
            match next {
                Some(c) => cursor = Some(c),
                None => break,
            }
        }
        Ok(ids)
    }

    async fn fetch_favorites_page(
        &self,
        cursor: Option<String>,
    ) -> Result<FavoritesPage, SourceError> {
        let token = self.token()?;
        let (tracks, next) =
            crate::spotify::api::saved_tracks_page(token, cursor.as_deref()).await?;
        Ok(FavoritesPage { tracks, next })
    }

    async fn push_favorite(&self, item_id: &str, on: bool) -> Result<(), SourceError> {
        let token = self.token()?;
        crate::spotify::api::set_saved(token, item_id, on)
            .await
            .map_err(SourceError::from)
    }

    async fn search(
        &self,
        query: &str,
    ) -> Result<(Vec<reader::Track>, Vec<reader::Album>), SourceError> {
        let token = self.token()?;
        crate::spotify::api::search(token, query)
            .await
            .map_err(SourceError::from)
    }

    async fn fetch_library(&self) -> Result<LibrarySnapshot, SourceError> {
        let token = self.token()?;
        let (albums, tracks) = crate::spotify::api::saved_albums(token).await?;
        Ok(LibrarySnapshot {
            albums,
            tracks,
            artist_images: Vec::new(),
        })
    }

    async fn fetch_playlists(&self) -> Result<Vec<PlaylistMeta>, SourceError> {
        let token = self.token()?;
        Ok(crate::spotify::api::list_playlists(token)
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
        let token = self.token()?;
        crate::spotify::api::playlist_entries(token, playlist_id)
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
