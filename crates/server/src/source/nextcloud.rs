use std::collections::HashMap;

use async_trait::async_trait;
use config::{MusicService, Source};
use db::Db;
use futures_util::StreamExt;

use crate::{
    nextcloud::{ArtTrack, NextcloudClient},
    server_ops::ServerConn,
};

use super::{
    AlbumType, ArtistView, AuthOutcome, Capabilities, FavoritesSync, LibrarySnapshot, MediaSource,
    PlaylistOps, RadioSeeds, SourceError, StreamInfo,
};

/// Item ids are remote paths: oc:fileid survives renames but needs a lookup to
/// address, and every URL here is built from the path anyway.
pub(super) struct NextcloudSource {
    db: Db,
    source: Source,
    client: Option<NextcloudClient>,
    folders: Vec<String>,
}

impl NextcloudSource {
    pub(super) fn new(db: Db, source: Source, conn: &ServerConn) -> Self {
        // A bad URL reports Connectivity from the ops, rather than failing here.
        let client = NextcloudClient::new(&conn.url, &conn.user_id, &conn.token)
            .inspect_err(|e| tracing::warn!(error = %e, "nextcloud client unavailable"))
            .ok();
        Self {
            db,
            source,
            client,
            folders: conn.folders.clone(),
        }
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
    radio: RadioSeeds::NONE,
    playlists: PlaylistOps::None, // none over raw WebDAV, the Music app's are Subsonic
    artist_view: ArtistView::Library,
    albums: AlbumType::Standard,
    favorites_sync: FavoritesSync::Instant,
};

/// Art fetches in flight at once, enough to hide the round trips without
/// loading someone's home instance like a scan.
const ART_CONCURRENCY: usize = 8;

/// Duration probes in flight at once, held back like the art fetches.
const DURATION_CONCURRENCY: usize = 8;

/// Cache one album's art: a sidecar image if there is one, else the picture the
/// tracks carry. Parts come by value because a future borrowing the album fails
/// lifetime inference once buffered inside an `async_trait` method.
async fn album_art(
    client: &NextcloudClient,
    sidecar: Option<String>,
    art_track: Option<ArtTrack>,
) -> Option<String> {
    let cached = match &sidecar {
        Some(remote) => client.cache_cover(remote).await,
        None => None,
    };
    match (cached, &art_track) {
        (None, Some(track)) => client.cache_track_art(track).await,
        (cached, _) => cached,
    }
    .map(|path| path.to_string_lossy().into_owned())
}

/// Track lengths by remote path, probed only for paths the last scan left
/// without one. A header stating no length leaves no mark to reuse, so such a
/// track is probed again every scan. Paths that stay unknown are absent.
async fn track_durations(
    client: &NextcloudClient,
    db: &Db,
    source: &Source,
    paths: &[String],
) -> HashMap<String, u64> {
    let mut durations: HashMap<String, u64> = db
        .tracks_by_keys(source, paths)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "could not reuse stored nextcloud durations");
            Vec::new()
        })
        .into_iter()
        .filter(|track| track.duration > 0)
        .map(|track| (track.id.key().into_owned(), track.duration))
        .collect();

    // Owned, for the same lifetime reason album_art takes owned parts.
    let missing: Vec<String> = paths
        .iter()
        .filter(|path| !durations.contains_key(*path))
        .cloned()
        .collect();

    let probed = futures_util::stream::iter(missing.into_iter().map(|path| async move {
        let secs = client.probe_duration(&path).await?;
        Some((path, secs))
    }))
    .buffered(DURATION_CONCURRENCY)
    .collect::<Vec<_>>()
    .await;

    durations.extend(probed.into_iter().flatten());
    durations
}

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
        let (albums, tracks) = client
            .scan(&self.folders)
            .await
            .map_err(SourceError::Backend)?;

        // Nothing persists until the snapshot is whole, so a serial pass here stalls the sync.
        let mut jobs = Vec::with_capacity(albums.len());
        for album in &albums {
            jobs.push(album_art(
                client,
                album.cover_path.clone(),
                album.art_track.clone(),
            ));
        }
        let covers = futures_util::stream::iter(jobs)
            .buffered(ART_CONCURRENCY)
            .collect::<Vec<_>>()
            .await;

        // Per album, before the tracks, so an album's tracks share one file.
        let mut cached_covers: HashMap<String, String> = HashMap::new();
        let mut out_albums = Vec::with_capacity(albums.len());

        for (album, cached) in albums.into_iter().zip(covers) {
            if let Some(path) = &cached {
                cached_covers.insert(album.path.clone(), path.clone());
            }

            out_albums.push(reader::Album {
                // No cover segment: a remote path can hold the ':' the ref splits on.
                id: reader::CoverRef::stored_item_ref(MusicService::Nextcloud, &album.path, None),
                title: album.title,
                artist: album.artist,
                genre: String::new(),
                year: 0,
                cover_path: cached.as_deref().map(PathBuf::from),
                manual_cover: false,
            });
        }

        let paths: Vec<String> = tracks.iter().map(|track| track.path.clone()).collect();
        let durations = track_durations(client, &self.db, &self.source, &paths).await;

        let out_tracks = tracks
            .into_iter()
            .map(|track| {
                let cached = cached_covers.get(&track.album_path).map(String::as_str);
                let duration = durations.get(&track.path).copied().unwrap_or(0);
                reader::Track {
                    id: reader::models::TrackId::Server {
                        service: MusicService::Nextcloud,
                        item_id: track.path,
                    },
                    cover: Some(cover_ref(cached).to_string()),
                    album_id: reader::CoverRef::stored_item_ref(
                        MusicService::Nextcloud,
                        &track.album_path,
                        None,
                    ),
                    title: track.title,
                    artist: track.artist.clone(),
                    album: track.album,
                    // 0 when neither the listing nor the header stated one.
                    duration,
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
            // The scan probed the header; repeating it costs a round trip per play.
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
            .favorites(&self.folders)
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

    /// A colon in the path would eat the cover segment of a three-part ref.
    #[test]
    fn album_ids_survive_a_colon_in_the_path() {
        let path = "/Music/Artist/Vol 1: Deluxe";
        let id = reader::CoverRef::stored_item_ref(MusicService::Nextcloud, path, None);
        assert_eq!(id, "nextcloud:/Music/Artist/Vol 1: Deluxe");

        // Art travels beside the id, as a path that parses on its own shape.
        let cached = "/home/u/.cache/kopuz/nextcloud-covers/abc.jpg";
        assert_eq!(
            reader::CoverRef::parse(cached),
            reader::CoverRef::Local(std::path::PathBuf::from(cached))
        );
    }

    #[test]
    fn client_rejects_empty_url() {
        assert!(NextcloudClient::new("", "alice", "app-pw").is_err());
    }
}
