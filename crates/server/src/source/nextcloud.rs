use async_trait::async_trait;
use config::{MusicService, Source};
use db::Db;

use crate::{nextcloud::NextcloudClient, server_ops::ServerConn};

use super::{
    AlbumType, ArtistView, AuthOutcome, Capabilities, FavoritesSync, LibrarySnapshot, MediaSource,
    PlaylistOps, SourceError, StreamInfo,
};

/// Item ids are remote paths: oc:fileid survives renames but needs a lookup to
/// address, and every URL here is built from the path anyway.
pub(super) struct NextcloudSource {
    db: Db,
    source: Source,
    client: Option<NextcloudClient>,
}

impl NextcloudSource {
    pub(super) fn new(db: Db, source: Source, conn: &ServerConn) -> Self {
        // A bad URL reports Connectivity from the ops, rather than failing here.
        let client = NextcloudClient::new(&conn.url, &conn.user_id, &conn.token)
            .inspect_err(|e| tracing::warn!(error = %e, "nextcloud client unavailable"))
            .ok();
        Self { db, source, client }
    }

    fn client(&self) -> Result<&NextcloudClient, SourceError> {
        self.client.as_ref().ok_or(SourceError::Connectivity)
    }
}

const CAPABILITIES: Capabilities = Capabilities {
    edit_tags: false,
    delete_from_disk: false,
    scan_folders: false,
    folders: false,
    sync: true,
    downloads: true,
    discover: false,
    radio: false,
    playlists: PlaylistOps::None, // none over raw WebDAV, the Music app's are Subsonic
    artist_view: ArtistView::Library,
    albums: AlbumType::Standard,
    favorites_sync: FavoritesSync::Instant,
};

/// Cached art, or the sentinel that stops the resolver guessing at a URL.
fn cover_ref(cached: Option<&str>) -> &str {
    cached.unwrap_or(reader::CoverRef::NO_COVER)
}

#[async_trait]
impl MediaSource for NextcloudSource {
    fn source(&self) -> &Source {
        &self.source
    }
    fn db(&self) -> &Db {
        &self.db
    }

    fn capabilities(&self) -> Capabilities {
        CAPABILITIES
    }

    async fn fetch_library(&self) -> Result<LibrarySnapshot, SourceError> {
        use std::path::PathBuf;

        let client = self.client()?;
        let (albums, tracks) = client.scan().await.map_err(SourceError::Backend)?;

        // Per album, before the tracks, so an album's tracks share one file.
        let mut cached_covers: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut out_albums = Vec::with_capacity(albums.len());

        for album in albums {
            let cached = match &album.cover_path {
                Some(remote) => client
                    .cache_cover(remote)
                    .await
                    .map(|p| p.to_string_lossy().into_owned()),
                None => None,
            };
            if let Some(path) = &cached {
                cached_covers.insert(album.path.clone(), path.clone());
            }

            out_albums.push(reader::Album {
                id: reader::CoverRef::stored_item_ref(
                    MusicService::Nextcloud,
                    &album.path,
                    Some(cover_ref(cached.as_deref())),
                ),
                title: album.title,
                artist: album.artist,
                genre: String::new(),
                year: 0,
                cover_path: cached.as_deref().map(PathBuf::from),
                manual_cover: false,
            });
        }

        let out_tracks = tracks
            .into_iter()
            .map(|track| {
                let cached = cached_covers.get(&track.album_path).map(String::as_str);
                reader::Track {
                    id: reader::models::TrackId::Server {
                        service: MusicService::Nextcloud,
                        item_id: track.path,
                    },
                    cover: Some(cover_ref(cached).to_string()),
                    album_id: reader::CoverRef::stored_item_ref(
                        MusicService::Nextcloud,
                        &track.album_path,
                        Some(cover_ref(cached)),
                    ),
                    title: track.title,
                    artist: track.artist.clone(),
                    album: track.album,
                    duration: 0, // WebDAV has none of these; the decode probe fills them
                    khz: 0,
                    bitrate: 0,
                    track_number: track.track_number,
                    disc_number: track.disc_number,
                    musicbrainz_release_id: None,
                    musicbrainz_recording_id: None,
                    musicbrainz_track_id: None,
                    playlist_item_id: None,
                    artists: vec![track.artist],
                }
            })
            .collect();

        Ok(LibrarySnapshot {
            albums: out_albums,
            tracks: out_tracks,
            artist_images: Vec::new(),
        })
    }

    async fn resolve_stream(&self, item_id: &str) -> Result<StreamInfo, SourceError> {
        Ok(StreamInfo {
            url: self.client()?.stream_url(item_id),
            format: None,
            user_agent: None,
            duration_secs: None,
            bitrate: None,
            content_length: None,
        })
    }

    async fn validate(&self) -> AuthOutcome {
        let Ok(client) = self.client() else {
            return AuthOutcome::Unreachable;
        };
        match client.ping().await {
            Ok(()) => AuthOutcome::Valid,
            Err(e) if e.is_auth_error() => AuthOutcome::Expired,
            Err(_) => AuthOutcome::Unreachable,
        }
    }

    async fn fetch_favorites(&self) -> Result<Vec<String>, SourceError> {
        self.client()?
            .favorites()
            .await
            .map_err(SourceError::Backend)
    }

    async fn push_favorite(&self, item_id: &str, on: bool) -> Result<(), SourceError> {
        self.client()?
            .set_favorite(item_id, on)
            .await
            .map_err(SourceError::Backend)
    }

    async fn add_to_playlist(
        &self,
        _playlist_id: &str,
        _item_refs: &[String],
    ) -> Result<Vec<String>, SourceError> {
        Err(SourceError::unsupported("playlists"))
    }

    async fn create_playlist(
        &self,
        _name: &str,
        _item_refs: &[String],
    ) -> Result<String, SourceError> {
        Err(SourceError::unsupported("playlists"))
    }

    async fn remove_from_playlist(
        &self,
        _playlist_id: &str,
        _track: &reader::Track,
        _position: usize,
    ) -> Result<(), SourceError> {
        Err(SourceError::unsupported("playlists"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_ref_falls_back_to_sentinel() {
        assert_eq!(cover_ref(None), reader::CoverRef::NO_COVER);
        assert_eq!(cover_ref(Some("/cache/a.jpg")), "/cache/a.jpg");
        assert_eq!(
            reader::CoverRef::parse(cover_ref(None)),
            reader::CoverRef::None
        );
    }

    #[test]
    fn client_rejects_empty_url() {
        assert!(NextcloudClient::new("", "alice", "app-pw").is_err());
    }
}
