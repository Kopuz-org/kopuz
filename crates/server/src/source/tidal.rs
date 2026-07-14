use async_trait::async_trait;
use config::Source;
use db::Db;

use crate::server_ops::ServerConn;
use crate::tidal::Creds;

use super::{
    AlbumType, ArtistView, AuthOutcome, Capabilities, FavoritesPage, FavoritesSync,
    LibrarySnapshot, MediaSource, PlaylistMeta, PlaylistOps, SourceError, StreamInfo,
};

pub(super) struct TidalSource {
    db: Db,
    source: Source,
    /// Unpacked device-flow credentials; `None` when the stored token isn't a
    /// packed TIDAL credential set (never signed in / legacy value).
    creds: Option<Creds>,
    /// TIDAL's numeric userId (the favorites/playlists path segment).
    user_id: String,
}

impl TidalSource {
    pub(super) fn new(db: Db, source: Source, conn: &ServerConn) -> Self {
        Self {
            db,
            source,
            creds: crate::tidal::unpack_creds(&conn.token),
            user_id: conn.user_id.clone(),
        }
    }

    fn creds(&self) -> Result<&Creds, SourceError> {
        self.creds.as_ref().ok_or(SourceError::Auth)
    }
}

#[async_trait]
impl MediaSource for TidalSource {
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
            // Read-only for now: playlist mutation isn't wired.
            playlists: PlaylistOps::None,
            artist_view: ArtistView::Library,
            albums: AlbumType::Standard,
            favorites_sync: FavoritesSync::Paginated,
        }
    }

    async fn resolve_stream(&self, item_id: &str) -> Result<StreamInfo, SourceError> {
        let url = crate::tidal::resolve_stream(self.creds()?, item_id).await?;
        Ok(StreamInfo {
            url,
            format: None,
            user_agent: None,
            duration_secs: None,
            bitrate: None,
            content_length: None,
        })
    }

    async fn validate(&self) -> AuthOutcome {
        let Some(creds) = self.creds.as_ref() else {
            return AuthOutcome::Expired;
        };
        match crate::tidal::get_session(creds).await {
            Ok(_) => AuthOutcome::Valid,
            // The ~7-day access token lapsed — the sign-in flow refreshes it.
            Err(e) if e.contains("401") => AuthOutcome::Expired,
            Err(_) => AuthOutcome::Unreachable,
        }
    }

    async fn fetch_library(&self) -> Result<LibrarySnapshot, SourceError> {
        let (albums, tracks, artist_images) =
            crate::tidal::fetch_library(self.creds()?, &self.user_id).await?;
        Ok(LibrarySnapshot {
            albums,
            tracks,
            artist_images,
        })
    }

    async fn fetch_favorites(&self) -> Result<Vec<String>, SourceError> {
        let creds = self.creds()?;
        let mut ids = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let (tracks, next) =
                crate::tidal::favorite_tracks_page(creds, &self.user_id, cursor.as_deref()).await?;
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
        let (tracks, next) =
            crate::tidal::favorite_tracks_page(self.creds()?, &self.user_id, cursor.as_deref())
                .await?;
        Ok(FavoritesPage { tracks, next })
    }

    async fn push_favorite(&self, item_id: &str, on: bool) -> Result<(), SourceError> {
        crate::tidal::set_track_favorite(self.creds()?, &self.user_id, item_id, on)
            .await
            .map_err(SourceError::from)
    }

    async fn search(
        &self,
        query: &str,
    ) -> Result<(Vec<reader::Track>, Vec<reader::Album>), SourceError> {
        let tracks = crate::tidal::search_tracks(self.creds()?, query).await?;
        Ok((tracks, Vec::new()))
    }

    async fn fetch_playlists(&self) -> Result<Vec<PlaylistMeta>, SourceError> {
        Ok(crate::tidal::list_playlists(self.creds()?, &self.user_id)
            .await?
            .into_iter()
            .map(|p| PlaylistMeta {
                id: p.id,
                name: p.title,
                image_tag: p.image_url,
            })
            .collect())
    }

    async fn fetch_playlist_entries(
        &self,
        playlist_id: &str,
    ) -> Result<Vec<reader::Track>, SourceError> {
        crate::tidal::get_playlist_entries(self.creds()?, playlist_id)
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
