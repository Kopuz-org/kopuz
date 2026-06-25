use async_trait::async_trait;
use config::Source;
use db::Db;

use crate::server_ops::ServerConn;

use super::{
    AlbumType, ArtistView, AuthOutcome, Capabilities, FavoritesPage, FavoritesSync, MediaSource,
    PlaylistMeta, PlaylistOps, SourceError, StreamInfo,
};

pub(super) struct SoundcloudSource {
    db: Db,
    source: Source,
    /// OAuth token for the signed-in account; `None` = anonymous (search + play
    /// of public tracks still work via the scraped web-player client_id).
    token: Option<String>,
}

impl SoundcloudSource {
    pub(super) fn new(db: Db, source: Source, conn: &ServerConn) -> Self {
        Self {
            db,
            source,
            token: (!conn.token.is_empty()).then(|| conn.token.clone()),
        }
    }
}

#[async_trait]
impl MediaSource for SoundcloudSource {
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
            // No write side wired (api-v2 playlist mutation is DataDome-gated).
            playlists: PlaylistOps::None,
            artist_view: ArtistView::Library,
            albums: AlbumType::Standard,
            favorites_sync: FavoritesSync::Paginated,
        }
    }

    async fn resolve_stream(&self, item_id: &str) -> Result<StreamInfo, SourceError> {
        let url = match crate::soundcloud::resolve_stream(item_id, self.token.as_deref()).await? {
            // Progressive MP3 streams straight through the normal HTTP path.
            crate::soundcloud::ResolvedStream::Progressive(u) => u,
            // HLS (Go+ AAC) is tagged so the player assembles its fMP4 segments
            // (Symphonia has no HLS demuxer) instead of streaming the .m3u8.
            crate::soundcloud::ResolvedStream::HlsAac(u) => format!("__SC_HLS:{u}"),
        };
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
        match self.token.as_deref() {
            // Anonymous mode is always usable (public search + play).
            None => AuthOutcome::Valid,
            Some(token) => match crate::soundcloud::get_me(token).await {
                Ok(_) => AuthOutcome::Valid,
                // Can't cleanly tell expired-token from a network blip, so don't
                // force a re-sign-in: treat any failure as unreachable.
                Err(_) => AuthOutcome::Unreachable,
            },
        }
    }

    async fn fetch_favorites(&self) -> Result<Vec<String>, SourceError> {
        let Some(token) = self.token.as_deref() else {
            return Ok(Vec::new());
        };
        let mut ids = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let (tracks, next) =
                crate::soundcloud::liked_tracks_page(token, cursor.as_deref()).await?;
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
        let Some(token) = self.token.as_deref() else {
            return Ok(FavoritesPage {
                tracks: Vec::new(),
                next: None,
            });
        };
        let (tracks, next) = crate::soundcloud::liked_tracks_page(token, cursor.as_deref()).await?;
        Ok(FavoritesPage { tracks, next })
    }

    async fn push_favorite(&self, item_id: &str, on: bool) -> Result<(), SourceError> {
        let token = self.token.as_deref().ok_or(SourceError::Auth)?;
        crate::soundcloud::set_track_like(item_id, on, token)
            .await
            .map_err(SourceError::from)
    }

    async fn search(
        &self,
        query: &str,
    ) -> Result<(Vec<reader::Track>, Vec<reader::Album>), SourceError> {
        let tracks = crate::soundcloud::search_tracks(query).await?;
        Ok((tracks, Vec::new()))
    }

    async fn fetch_playlists(&self) -> Result<Vec<PlaylistMeta>, SourceError> {
        let Some(token) = self.token.as_deref() else {
            return Ok(Vec::new());
        };
        Ok(crate::soundcloud::list_playlists(token)
            .await?
            .into_iter()
            .map(|p| PlaylistMeta {
                id: p.id,
                name: p.title,
                image_tag: p.artwork_url,
            })
            .collect())
    }

    async fn fetch_playlist_entries(
        &self,
        playlist_id: &str,
    ) -> Result<Vec<reader::Track>, SourceError> {
        let Some(token) = self.token.as_deref() else {
            return Ok(Vec::new());
        };
        crate::soundcloud::get_playlist_entries(playlist_id, token)
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
