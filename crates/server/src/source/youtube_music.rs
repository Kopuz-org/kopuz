use async_trait::async_trait;
use config::Source;
use db::Db;

use crate::{server_ops::ServerConn, ytmusic::YouTubeMusicClient};

use super::{
    AlbumType, ArtistView, AuthOutcome, Capabilities, FavoritesPage, FavoritesSync, MediaSource,
    PlaylistMeta, PlaylistOps, PlaylistPage, RemoteAlbum, SourceError, StreamInfo, mirror_added,
    mirror_created,
};

pub(super) struct YtSource {
    db: Db,
    source: Source,
    client: YouTubeMusicClient,
}

impl YtSource {
    pub(super) fn new(db: Db, source: Source, conn: &ServerConn) -> Self {
        Self {
            db,
            source,
            client: YouTubeMusicClient::with_cookies(conn.token.clone()),
        }
    }
}

#[async_trait]
impl MediaSource for YtSource {
    fn source(&self) -> &Source {
        &self.source
    }
    fn db(&self) -> &Db {
        &self.db
    }

    fn capabilities(&self) -> Capabilities {
        // YT has discover + radio, can add/remove playlist entries, but its
        // InnerTube exposes no reorder mutation — so no `reorder_playlist`
        // override; it inherits the unsupported default.
        Capabilities {
            edit_tags: false,
            delete_from_disk: false,
            scan_folders: false,
            folders: false,
            sync: true,
            downloads: true,
            discover: true,
            radio: true,
            playlists: PlaylistOps::AddRemove,
            artist_view: ArtistView::Remote,
            albums: AlbumType::YtMusic,
            favorites_sync: FavoritesSync::Paginated,
        }
    }

    async fn start_radio(&self, seed_ref: &str) -> Result<Vec<reader::Track>, SourceError> {
        if seed_ref.trim().is_empty() {
            return Err(SourceError::InvalidInput("track has no video id".into()));
        }
        // /next works anonymously (empty cookies), so no auth gate here.
        self.client
            .start_mix(seed_ref)
            .await
            .map_err(SourceError::from)
    }

    async fn web_url(&self, track: &reader::Track) -> Option<String> {
        let vid = track.id.key();
        (!vid.trim().is_empty()).then(|| format!("https://music.youtube.com/watch?v={vid}"))
    }

    async fn search(
        &self,
        query: &str,
    ) -> Result<(Vec<reader::Track>, Vec<reader::Album>), SourceError> {
        if query.trim().is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }
        let tracks = self.client.search_tracks(query).await?;
        Ok((tracks, Vec::new()))
    }

    async fn discover_home(&self) -> Result<crate::ytmusic::discover::DiscoverHome, SourceError> {
        self.client.discover_home().await.map_err(SourceError::from)
    }

    async fn discover_continuation(
        &self,
        token: &str,
    ) -> Result<crate::ytmusic::discover::DiscoverHome, SourceError> {
        self.client
            .discover_continuation(token)
            .await
            .map_err(SourceError::from)
    }

    async fn fetch_album_tracks(&self, browse_id: &str) -> Result<Vec<reader::Track>, SourceError> {
        self.client
            .fetch_album_tracks(browse_id)
            .await
            .map_err(SourceError::from)
    }

    async fn fetch_album(&self, browse_id: &str) -> Result<RemoteAlbum, SourceError> {
        self.client
            .fetch_album(browse_id)
            .await
            .map(RemoteAlbum::from)
            .map_err(SourceError::from)
    }

    async fn fetch_album_by_ref(&self, id: &str) -> Result<Option<RemoteAlbum>, SourceError> {
        // Resolve the id (raw browse id, `ytmusic:album:MPRE…`, or a synthesized
        // `ytmusic:album:<hash>`) to a real browse id before fetching.
        let browse_id = if let Some(bid) = crate::ytmusic::search::album_browse_id(id) {
            Some(bid)
        } else if let Some((album, artist)) = crate::ytmusic::search::synth_album_parts(id) {
            self.resolve_album_browse_id(&album, &artist).await?
        } else {
            None
        };
        let Some(browse_id) = browse_id else {
            return Ok(None);
        };
        Ok(self
            .fetch_album(&browse_id)
            .await
            .ok()
            .filter(|a| !a.tracks.is_empty()))
    }

    async fn fetch_album_by_meta(
        &self,
        title: &str,
        artist: &str,
    ) -> Result<Option<RemoteAlbum>, SourceError> {
        let Some(browse_id) = self.resolve_album_browse_id(title, artist).await? else {
            return Ok(None);
        };
        Ok(self
            .fetch_album(&browse_id)
            .await
            .ok()
            .filter(|a| !a.tracks.is_empty()))
    }

    async fn fetch_playlist_page(
        &self,
        playlist_id: &str,
        cursor: Option<String>,
    ) -> Result<(Vec<reader::Track>, Option<String>), SourceError> {
        self.client
            .playlist_page(playlist_id, cursor.as_deref())
            .await
            .map_err(SourceError::from)
    }

    async fn resolve_artist_channel_id(&self, query: &str) -> Result<Option<String>, SourceError> {
        self.client
            .resolve_artist_channel_id(query)
            .await
            .map_err(SourceError::from)
    }

    async fn resolve_album_browse_id(
        &self,
        album: &str,
        artist: &str,
    ) -> Result<Option<String>, SourceError> {
        self.client
            .resolve_album_browse_id(album, artist)
            .await
            .map_err(SourceError::from)
    }

    async fn fetch_artist(
        &self,
        channel_id: &str,
    ) -> Result<crate::ytmusic::discover::YtArtist, SourceError> {
        self.client
            .fetch_artist(channel_id)
            .await
            .map_err(SourceError::from)
    }

    async fn fetch_artist_image(&self, name: &str) -> Result<Option<String>, SourceError> {
        self.client
            .resolve_artist_image(name)
            .await
            .map_err(SourceError::from)
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
        let vid = track.id.key();
        if vid.is_empty() {
            return Err(SourceError::InvalidInput("track has no video id".into()));
        }
        self.client.remove_from_playlist(playlist_id, &vid).await?;
        self.db
            .remove_playlist_tracks(&self.source, playlist_id, &[vid.into_owned()])
            .await
            .map_err(SourceError::from)
    }

    async fn resolve_stream(&self, item_id: &str) -> Result<StreamInfo, SourceError> {
        let info = self.client.get_stream(item_id).await?;
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
        match self.client.validate_cookies().await {
            Ok(()) => AuthOutcome::Valid,
            Err(e) if e.contains("cookies expired") || e.contains("signed out") => {
                AuthOutcome::Expired
            }
            Err(_) => AuthOutcome::Unreachable,
        }
    }

    async fn fetch_favorites(&self) -> Result<Vec<String>, SourceError> {
        let mut ids = Vec::new();
        self.client
            .stream_liked_songs(|page| {
                ids.extend(page.into_iter().map(|t| t.id.key().into_owned()));
            })
            .await?;
        Ok(ids)
    }

    async fn push_favorite(&self, item_id: &str, on: bool) -> Result<(), SourceError> {
        if on {
            self.client.like_video(item_id).await
        } else {
            self.client.unlike_video(item_id).await
        }
        .map_err(SourceError::from)
    }

    async fn fetch_playlists(&self) -> Result<Vec<PlaylistMeta>, SourceError> {
        Ok(self
            .client
            .list_playlists()
            .await?
            .into_iter()
            .map(|s| PlaylistMeta {
                id: s.id,
                name: s.title,
                image_tag: s
                    .thumbnail_url
                    .as_ref()
                    .map(|u| utils::jellyfin_image::encode_cover_url(u)),
            })
            .collect())
    }

    async fn fetch_playlist_entries(
        &self,
        playlist_id: &str,
    ) -> Result<Vec<reader::Track>, SourceError> {
        // The YT client already returns typed tracks.
        Ok(self.client.get_playlist_entries(playlist_id).await?)
    }

    async fn fetch_playlist_entries_page(
        &self,
        playlist_id: &str,
        cursor: Option<String>,
    ) -> Result<PlaylistPage, SourceError> {
        // True per-page InnerTube walk so a long playlist streams into the cache
        // (and the UI) instead of blocking on a full fetch every visit.
        let (tracks, next) = self
            .client
            .playlist_page(playlist_id, cursor.as_deref())
            .await?;
        Ok(PlaylistPage { tracks, next })
    }

    async fn fetch_favorites_page(
        &self,
        cursor: Option<String>,
    ) -> Result<FavoritesPage, SourceError> {
        let (tracks, next) = self.client.liked_songs_page(cursor.as_deref()).await?;
        Ok(FavoritesPage { tracks, next })
    }
}
