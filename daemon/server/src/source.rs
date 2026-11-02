//! Media backends and source-scoped cache operations.
//!
//! [`MediaSource`] implementations handle remote requests and declare supported
//! operations through [`Capabilities`]. Default cache methods use the backend's
//! source partition. [`resolve`] constructs the backend for a source.

use async_trait::async_trait;

use config::{MusicService, Source};
use db::Db;

use crate::server_ops::ServerConn;
use crate::sources::Sources;

mod apple_music;
mod audio_cd;
mod clippsly;
mod jellyfin;
mod local;
mod nextcloud;
mod offline;
mod search;
mod smb;
mod soundcloud;
mod spotify;
mod subsonic;
mod types;
mod youtube_music;
use apple_music::AppleMusicSource;
use clippsly::ClippslySource;
use jellyfin::JellyfinSource;
use local::LocalSource;
use nextcloud::NextcloudSource;
use offline::OfflineServerSource;
use smb::SmbSource;
use soundcloud::SoundcloudSource;
use spotify::SpotifySource;
use subsonic::SubsonicSource;
pub use types::*;
use youtube_music::YtSource;

/// The source-agnostic backend the app drives. Impls supply
/// [`source`](Self::source) + [`db`](Self::db) (the seam the default methods
/// build on), [`capabilities`](Self::capabilities), and the remote-reaching
/// ops; the uniform DB ops are inherited.
#[async_trait]
pub trait MediaSource: Send + Sync {
    /// The typed source this acts on; its [`as_str`](Source::as_str) is the
    /// favorites partition key (`"local"` or the server id).
    fn source(&self) -> &Source;

    /// The storage handle the default methods write through. An impl seam, not
    /// a call-site API — go through the operation methods, not the raw `Db`.
    #[doc(hidden)]
    fn db(&self) -> &Db;

    /// What this source supports — gated on by the UI (no `is_server()` split).
    fn capabilities(&self) -> Capabilities;

    /// Extract physical audio tracks to permanent files.
    fn rip_audio(&self) -> bool {
        false
    }

    // --- remote-reaching ops (required) -------------------------------------

    /// Append refs to an existing playlist. Local writes the DB; a server calls
    /// the remote and mirrors what landed into the DB cache. Returns the refs
    /// that landed.
    async fn add_to_playlist(
        &self,
        playlist_id: &str,
        item_refs: &[String],
    ) -> Result<Vec<String>, SourceError>;

    /// Create a playlist seeded with `item_refs`, returning its id (a fresh
    /// UUID for local, the remote's id for a server, also mirrored into the DB).
    async fn create_playlist(
        &self,
        name: &str,
        item_refs: &[String],
    ) -> Result<String, SourceError>;

    /// Remove the entry at `position`; the track carries that entry's own `playlist_item_id`.
    async fn remove_from_playlist(
        &self,
        playlist_id: &str,
        track: &reader::Track,
        position: usize,
    ) -> Result<(), SourceError>;

    /// Resolve a playable stream for one item id (local = a file path, server =
    /// the remote's URL / deciphered stream). One-format sources ignore `quality`.
    async fn resolve_stream(
        &self,
        item_id: &str,
        quality: config::StreamQuality,
    ) -> Result<StreamInfo, SourceError>;

    /// A seekable protocol-native stream, or `None` to use the resolved URL.
    async fn open_stream(
        &self,
        _item_id: &str,
        _progress: Option<crate::stream::stream_buffer::BufferProgressCallback>,
    ) -> Result<Option<Box<dyn symphonia::core::io::MediaSource>>, SourceError> {
        Ok(None)
    }

    /// The bytes of one track, for sources that can't express it as a URL.
    ///
    /// Almost nothing needs this: a download is normally a GET of whatever
    /// [`resolve_stream`](Self::resolve_stream) hands out, and that is what the
    /// unsupported default here leaves the caller to do. Apple Music overrides
    /// it because its assets are encrypted — the URL yields ciphertext, so a
    /// playable file only exists after a licence and a decrypt pass.
    async fn download_track(
        &self,
        _item_id: &str,
        _progress: Option<crate::stream::stream_buffer::BufferProgressCallback>,
    ) -> Result<Vec<u8>, SourceError> {
        Err(SourceError::unsupported("track download"))
    }

    /// Check stored creds against the source (local is always [`Valid`](AuthOutcome::Valid)).
    async fn validate(&self) -> AuthOutcome;

    /// Each step of reaching this source and playing from it, for a
    /// connection test. A source with nothing more to show reports its
    /// sign-in check alone.
    async fn diagnose(&self) -> Vec<Check> {
        let result = match self.validate().await {
            AuthOutcome::Valid => Ok(()),
            AuthOutcome::Expired => Err("the sign-in is no longer accepted".to_string()),
            AuthOutcome::Unreachable => Err("the server did not answer".to_string()),
        };
        vec![Check::of(CheckStep::Session, result, |()| {
            "accepted".to_string()
        })]
    }

    async fn storage_quota(&self) -> Result<StorageQuota, SourceError> {
        Err(SourceError::unsupported("storage quota"))
    }

    async fn upload_track(&self, _filename: String, _content: Vec<u8>) -> Result<(), SourceError> {
        Err(SourceError::unsupported("music uploads"))
    }

    /// All favorited item ids on the remote (YT pages internally). Local has no
    /// remote set, so it returns empty.
    async fn fetch_favorites(&self) -> Result<Vec<String>, SourceError>;

    /// Push one favorite to the remote (the reconciler's flush). A no-op for
    /// local (its favorites are already the DB rows).
    async fn push_favorite(&self, item_id: &str, on: bool) -> Result<(), SourceError>;

    /// Persist a playlist reorder: `ordered` is the full new membership, entry ids kept;
    /// `moved`/`new_index` identify the one entry that changed position. Only
    /// sources whose [`Capabilities::playlists`] is [`PlaylistOps::Reorder`]
    /// override this; the rest inherit the unsupported default.
    async fn reorder_playlist(
        &self,
        _playlist_id: &str,
        _ordered: &[reader::PlaylistEntry],
        _moved: &reader::Track,
        _new_index: usize,
    ) -> Result<(), SourceError> {
        Err(SourceError::unsupported("playlist reorder"))
    }

    /// The other cut of `track`, when the source pairs them and the track does not say.
    /// `None` is an answer, not a failure: most tracks have no counterpart.
    async fn counterpart(
        &self,
        _track: &reader::Track,
    ) -> Result<Option<reader::Counterpart>, SourceError> {
        Ok(None)
    }

    /// A video track's picture stream, which the daemon fetches by range for a frontend.
    async fn video_stream(&self, _item_id: &str) -> Result<VideoStream, SourceError> {
        Err(SourceError::unsupported("music videos"))
    }

    /// Tell the source to stop recommending `item_id` — YT Music's dislike,
    /// which is a signal to its recommender rather than a library edit. Only
    /// sources whose [`Capabilities::dont_recommend`] is set override this; the
    /// rest inherit the unsupported default.
    async fn dont_recommend(&self, _item_id: &str) -> Result<(), SourceError> {
        Err(SourceError::unsupported("don't recommend"))
    }

    async fn accounts(&self) -> Result<Vec<SourceAccount>, SourceError> {
        Err(SourceError::unsupported("accounts"))
    }

    /// `item_ref` is a ref this source handed out, or a track's key.
    async fn rate(
        &self,
        _item_ref: &str,
        _rating: crate::ytmusic::discover::Rating,
    ) -> Result<(), SourceError> {
        Err(SourceError::unsupported("rating"))
    }

    async fn follow(&self, _artist_ref: &str, _follow: bool) -> Result<(), SourceError> {
        Err(SourceError::unsupported("following"))
    }

    async fn save(&self, _item_ref: &str, _saved: bool) -> Result<(), SourceError> {
        Err(SourceError::unsupported("saving to the library"))
    }

    async fn remove_from_history(&self, _token: &str) -> Result<(), SourceError> {
        Err(SourceError::unsupported("history removal"))
    }

    async fn edit_playlist(
        &self,
        _playlist_id: &str,
        _details: &PlaylistDetails,
    ) -> Result<(), SourceError> {
        Err(SourceError::unsupported("playlist details"))
    }

    /// Runs before the local delete. Kept apart from `delete_playlist`, which the
    /// sync's sweep also calls for playlists the remote already dropped.
    async fn delete_remote_playlist(&self, _playlist_id: &str) -> Result<(), SourceError> {
        Ok(())
    }

    /// Start a radio/mix seeded from a track, returning the generated queue. Only
    /// sources whose [`Capabilities::radio`] is set override this; the rest
    /// inherit the unsupported default.
    async fn start_radio(&self, _seed_ref: &str) -> Result<RadioPage, SourceError> {
        Err(SourceError::unsupported("radio"))
    }

    /// Start a radio/mix seeded from a whole playlist, returning the generated
    /// queue. The queue is the source's own mix for that playlist — not the
    /// playlist's tracks — so callers play it as-is. Shares
    /// [`Capabilities::radio`] with [`start_radio`](Self::start_radio): a source
    /// that can seed radio from a track can seed it from a playlist too.
    async fn start_playlist_radio(&self, _playlist_ref: &str) -> Result<RadioPage, SourceError> {
        Err(SourceError::unsupported("playlist radio"))
    }

    /// Only a source that hands out a cursor is ever asked.
    async fn more_radio(&self, _cursor: &str) -> Result<RadioPage, SourceError> {
        Err(SourceError::unsupported("radio continuation"))
    }

    /// Whether the server now holds `queue`. A queue holding tracks the source
    /// does not own has no remote form, so an impl skips it (`false`) rather
    /// than saving a partial one. An empty queue clears the server's.
    async fn save_play_queue(
        &self,
        _queue: &[reader::Track],
        _current_index: usize,
        _position_ms: u64,
    ) -> Result<bool, SourceError> {
        Err(SourceError::unsupported("play queue sync"))
    }

    async fn get_play_queue(&self) -> Result<Option<RemotePlayQueue>, SourceError> {
        Err(SourceError::unsupported("play queue sync"))
    }

    /// The track's canonical public web URL, when this source has shareable web
    /// pages (e.g. a YouTube Music watch link or Spotify track link). `None` otherwise — callers fall
    /// back to a metadata lookup (MusicBrainz). Sync: it's a pure id→URL mapping.
    fn web_url(&self, _track: &reader::Track) -> Option<String> {
        None
    }

    /// The same for an album, given the id this source browses it by. `None`
    /// when the source has no album pages, in which case a caller shares the
    /// first track's page instead.
    fn album_web_url(&self, _browse_id: &str) -> Option<String> {
        None
    }

    fn artist_web_url(&self, _artist_id: &str) -> Option<String> {
        None
    }

    fn playlist_web_url(&self, _playlist_id: &str) -> Option<String> {
        None
    }

    /// For a key nothing has listed this session; `None` when the source has no such track.
    async fn fetch_track(&self, _item_id: &str) -> Result<Option<reader::Track>, SourceError> {
        Ok(None)
    }

    /// `None` anonymously, and for a source whose accounts have no picture.
    async fn account_avatar(&self) -> Result<Option<String>, SourceError> {
        Ok(None)
    }

    /// Search this source for `query`, returning matching tracks and albums. The
    /// default searches the source's library corpus (the behavior local, Jellyfin
    /// and Subsonic all share); catalog-backed remotes (YT) override to query the
    /// remote instead. Every source supports search — there is no unsupported case.
    async fn search(
        &self,
        query: &str,
    ) -> Result<(Vec<reader::Track>, Vec<reader::Album>), SourceError> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }
        let tracks = self.db().search_corpus(self.source()).await?;
        let albums = self.db().albums(self.source()).await?;
        Ok(search::filter(&q, tracks, albums))
    }

    fn search_filters(&self) -> Vec<SearchFilterEntry> {
        Vec::new()
    }

    /// Shelves for an "all" filter, one long shelf under any other.
    async fn search_shelves(
        &self,
        _query: &str,
        _filter: &str,
        _continuation: Option<&str>,
    ) -> Result<crate::ytmusic::browse::search::SearchPage, SourceError> {
        Err(SourceError::unsupported("search filters"))
    }

    async fn search_suggestions(
        &self,
        _query: &str,
    ) -> Result<Vec<crate::ytmusic::browse::search::Suggestion>, SourceError> {
        Ok(Vec::new())
    }

    /// The discover/home feed. Default unsupported — gated by
    /// [`Capabilities::discover`]; only catalog remotes (YT) override.
    async fn discover_home(&self) -> Result<crate::ytmusic::discover::DiscoverHome, SourceError> {
        Err(SourceError::unsupported("discover"))
    }

    /// The next page of the discover feed for a continuation token. Default
    /// unsupported (see [`discover_home`](Self::discover_home)).
    async fn discover_continuation(
        &self,
        _token: &str,
    ) -> Result<crate::ytmusic::discover::DiscoverHome, SourceError> {
        Err(SourceError::unsupported("discover"))
    }

    fn catalog_pages(&self) -> Vec<CatalogPageEntry> {
        Vec::new()
    }

    /// The mixes the source made for this account, as playlist tiles; a source that makes none has none.
    async fn personal_mixes(
        &self,
    ) -> Result<Vec<crate::ytmusic::discover::DiscoverItem>, SourceError> {
        Ok(Vec::new())
    }

    /// A page by an id that `catalog_pages`, a chip or a link handed out.
    async fn browse_page(
        &self,
        _id: &str,
        _continuation: Option<&str>,
    ) -> Result<crate::ytmusic::discover::BrowsePage, SourceError> {
        Err(SourceError::unsupported("catalog pages"))
    }

    /// Similar songs, artists and albums for one of its tracks.
    async fn related(
        &self,
        _item_id: &str,
    ) -> Result<crate::ytmusic::discover::BrowsePage, SourceError> {
        Err(SourceError::unsupported("related"))
    }

    /// The tracks of a remote album / browse id. Standard library remotes use
    /// this to open or act on an album before a full cache sync; catalog remotes
    /// use it for discover surfaces. Default unsupported.
    async fn fetch_album_tracks(
        &self,
        _browse_id: &str,
    ) -> Result<Vec<reader::Track>, SourceError> {
        Err(SourceError::unsupported("album tracks"))
    }

    /// The full remote album (header metadata + tracks) for the YT-Music-style
    /// album page. Default unsupported; only catalog remotes (YT) override.
    async fn fetch_album(&self, _browse_id: &str) -> Result<RemoteAlbum, SourceError> {
        Err(SourceError::unsupported("album"))
    }

    /// The full remote album an opened album id names, or `None` when it names none with tracks.
    async fn fetch_album_by_ref(&self, _id: &str) -> Result<Option<RemoteAlbum>, SourceError> {
        Err(SourceError::unsupported("album by ref"))
    }

    /// One page of a remote playlist: `cursor = None` for the first page, then
    /// the returned cursor for each next (`None` once exhausted). Lets a UI loop
    /// stream a long playlist (play page 1 instantly, queue the rest) without a
    /// non-`Send` callback. Default unsupported; only catalog remotes (YT)
    /// override. Cross-page dedup is the caller's job.
    async fn fetch_playlist_page(
        &self,
        _playlist_id: &str,
        _cursor: Option<String>,
    ) -> Result<(Vec<reader::Track>, Option<String>), SourceError> {
        Err(SourceError::unsupported("playlist paging"))
    }

    /// A remote artist profile (banner, top songs, albums, related) by channel
    /// id. Default unsupported; only catalog remotes (YT) override.
    async fn fetch_artist(
        &self,
        _channel_id: &str,
    ) -> Result<crate::ytmusic::discover::YtArtist, SourceError> {
        Err(SourceError::unsupported("artist profile"))
    }

    /// Pull the source's entire remote library (albums, tracks, artist images),
    /// transformed into model types. Default empty — only library remotes
    /// (Jellyfin/Subsonic) override; the caller persists + prunes the result.
    async fn fetch_library(&self) -> Result<LibrarySnapshot, SourceError> {
        Ok(LibrarySnapshot::default())
    }

    /// Fetch a library while reporting the file currently being scanned.
    async fn fetch_library_with_progress(
        &self,
        _on_progress: std::sync::Arc<dyn Fn(String) + Send + Sync>,
    ) -> Result<LibrarySnapshot, SourceError> {
        self.fetch_library().await
    }

    /// Resolve missing album covers after the library has been persisted.
    async fn fetch_missing_covers(
        &self,
        _albums: &[reader::Album],
        _tracks: &[reader::Track],
        _on_progress: std::sync::Arc<dyn Fn(String) + Send + Sync>,
    ) -> Result<Vec<reader::Album>, SourceError> {
        Ok(Vec::new())
    }

    /// Fetch a playlist's tracks from the remote. Local playlists resolve their
    /// refs from the DB, so the default is empty — only server impls override.
    async fn fetch_playlist_entries(
        &self,
        _playlist_id: &str,
    ) -> Result<Vec<reader::Track>, SourceError> {
        Ok(Vec::new())
    }

    /// One page of a playlist's entries, for the streaming reconcile. The caller
    /// passes `None` then each returned cursor. The default returns all entries in
    /// a single page (`next = None`) via [`fetch_playlist_entries`] — fine for
    /// sources whose listing isn't paginated (Jellyfin/Subsonic); YT overrides it
    /// with a true per-page InnerTube walk so a 700-track list streams in.
    async fn fetch_playlist_entries_page(
        &self,
        playlist_id: &str,
        _cursor: Option<String>,
    ) -> Result<PlaylistPage, SourceError> {
        Ok(PlaylistPage {
            tracks: self.fetch_playlist_entries(playlist_id).await?,
            next: None,
            header: None,
        })
    }

    /// Fetch the source's playlists (id, name, image tag) from the remote — the
    /// listing only; entries come from [`fetch_playlist_entries`]. Default empty.
    async fn fetch_playlists(&self) -> Result<Vec<PlaylistMeta>, SourceError> {
        Ok(Vec::new())
    }

    /// The remote's own artist listing: each artist as it names itself, with its photo URL when it has one.
    /// Default empty (local reads them from the DB; YT has none).
    async fn fetch_artists(
        &self,
    ) -> Result<Vec<(reader::ArtistCredit, Option<String>)>, SourceError> {
        Ok(Vec::new())
    }

    /// Look one artist up for its photo, and for its name when the lookup went by the id the source issued.
    /// Default finds nothing; the catalog remote (YT) implements it so the Artists grid can show real YT photos.
    async fn fetch_artist_image(
        &self,
        _artist: &reader::ArtistCredit,
    ) -> Result<ArtistLookup, SourceError> {
        Ok(ArtistLookup::default())
    }

    /// One page of favorites — for [`FavoritesSync::Paginated`] sources (YT). The
    /// caller passes `None` then each returned cursor; default is a single empty
    /// page (instant sources use [`fetch_favorites`] instead).
    async fn fetch_favorites_page(
        &self,
        _cursor: Option<String>,
    ) -> Result<FavoritesPage, SourceError> {
        Ok(FavoritesPage {
            tracks: Vec::new(),
            next: None,
        })
    }

    async fn album_tracks(&self, album_id: &str) -> Result<Vec<reader::Track>, SourceError> {
        let cached = self
            .db()
            .album_tracks(self.source(), album_id)
            .await
            .map_err(SourceError::from)?;
        let caps = self.capabilities();
        if caps.sync && caps.albums == AlbumType::Standard {
            match self.fetch_album_tracks(album_id).await {
                Ok(remote) if !remote.is_empty() => return Ok(remote),
                Ok(_) | Err(SourceError::Unsupported(_)) => {}
                Err(error) => {
                    tracing::debug!(%error, %album_id, "remote album lookup failed; using cache");
                }
            }
        }
        Ok(cached)
    }

    /// This source's favorite refs — its partition of the favorites table. The
    /// partition key is the source's own identity, so callers never compute it.
    async fn favorites(&self) -> Result<Vec<String>, SourceError> {
        self.db()
            .favorites(self.source().as_str())
            .await
            .map_err(SourceError::from)
    }

    /// Whether `ref_` is currently favorited for this source.
    async fn is_favorite(&self, ref_: &str) -> bool {
        self.db()
            .is_favorite(self.source().as_str(), ref_)
            .await
            .unwrap_or(false)
    }

    /// Toggle a favorite — always a local DB write (optimistic, offline-capable):
    /// for a server source the row is marked pending and the reconciler pushes
    /// it via [`push_favorite`](Self::push_favorite) once the server is reachable.
    async fn set_favorite(&self, ref_: &str, on: bool) -> Result<(), SourceError> {
        self.db()
            .set_favorite(self.source().as_str(), ref_, on)
            .await
            .map_err(SourceError::from)
    }

    /// Record `track`'s favorite state LOCALLY as a clean row (no dirty/pending),
    /// without touching the remote. The optimistic half of a favorite toggle:
    /// write this first so the UI reflects the change immediately, then
    /// [`push_favorite`](Self::push_favorite) to the remote — and call this again
    /// with the opposite `on` to revert if the push is rejected. An empty key is a
    /// no-op.
    async fn record_favorite(&self, track: &reader::Track, on: bool) -> Result<(), SourceError> {
        let key = track.id.key();
        if key.trim().is_empty() {
            return Ok(());
        }

        if on {
            let _ = self.upsert_tracks(std::slice::from_ref(track)).await;
        }
        let sid = self.source().as_str();
        self.db().set_favorite(sid, key.as_ref(), on).await?;
        self.db()
            .clear_favorite_dirty(sid, key.as_ref())
            .await
            .map_err(SourceError::from)
    }

    /// Report a track as now-playing to the remote's scrobble endpoint. Default
    /// no-op — only remotes that scrobble (Subsonic) override. Best-effort: the
    /// caller ignores the result.
    async fn scrobble_now_playing(&self, _item_id: &str) -> Result<(), SourceError> {
        Ok(())
    }

    /// Report a track as played (submission scrobble). Default no-op; only
    /// scrobbling remotes (Subsonic) override.
    async fn scrobble(&self, _item_id: &str) -> Result<(), SourceError> {
        Ok(())
    }

    /// Keep the remote session warm (periodic ping). Default no-op; only remotes
    /// with sessions (Jellyfin) override.
    async fn keepalive(&self) -> Result<(), SourceError> {
        Ok(())
    }

    /// Report playback start of `item_id` to the remote's session API. Default
    /// no-op; only session remotes (Jellyfin) override.
    async fn report_playback_start(&self, _item_id: &str) -> Result<(), SourceError> {
        Ok(())
    }

    /// Report playback stopped at `position_ticks` (100ns units). Default no-op;
    /// only session remotes (Jellyfin) override.
    async fn report_playback_stopped(
        &self,
        _item_id: &str,
        _position_ticks: u64,
    ) -> Result<(), SourceError> {
        Ok(())
    }

    /// Report playback progress at `position_ticks` (100ns units). Default no-op;
    /// only session remotes (Jellyfin) override.
    async fn report_playback_progress(
        &self,
        _item_id: &str,
        _position_ticks: u64,
        _is_paused: bool,
    ) -> Result<(), SourceError> {
        Ok(())
    }

    /// Replace one playlist's membership (reorders, full rebuilds). DB-cache op.
    async fn set_playlist_tracks(
        &self,
        playlist_id: &str,
        entries: &[reader::PlaylistEntry],
    ) -> Result<(), SourceError> {
        self.db()
            .set_playlist_tracks(self.source(), playlist_id, entries)
            .await
            .map_err(SourceError::from)
    }

    /// Remove the entry at `index` in play order. DB-cache op.
    async fn remove_playlist_entry(
        &self,
        playlist_id: &str,
        index: usize,
    ) -> Result<(), SourceError> {
        self.db()
            .remove_playlist_entry(self.source(), playlist_id, index)
            .await
            .map_err(SourceError::from)
    }

    /// Remove refs from one playlist. DB-cache op.
    async fn remove_playlist_tracks(
        &self,
        playlist_id: &str,
        refs: &[String],
    ) -> Result<(), SourceError> {
        self.db()
            .remove_playlist_tracks(self.source(), playlist_id, refs)
            .await
            .map_err(SourceError::from)
    }

    /// Delete one playlist (membership cascades). DB-cache op.
    async fn delete_playlist(&self, playlist_id: &str) -> Result<(), SourceError> {
        self.db()
            .delete_playlist(self.source(), playlist_id)
            .await
            .map_err(SourceError::from)
    }

    /// Set a playlist's cover from a local image file. The default records the
    /// path as the DB cover; sources backed by a remote that stores playlist
    /// artwork (Jellyfin) override to push the image upstream first.
    async fn set_playlist_cover(
        &self,
        playlist_id: &str,
        name: &str,
        image_path: &std::path::Path,
        image_tag: Option<&str>,
    ) -> Result<(), SourceError> {
        let cover = image_path.to_string_lossy();
        self.db()
            .upsert_playlist_meta(self.source(), playlist_id, name, Some(&cover), image_tag)
            .await
            .map_err(SourceError::from)
    }

    /// Upsert this source's tracks (e.g. after a metadata edit). DB-cache op.
    async fn upsert_tracks(&self, tracks: &[reader::Track]) -> Result<(), SourceError> {
        self.db()
            .upsert_tracks(self.source(), tracks)
            .await
            .map_err(SourceError::from)
    }

    /// Delete this source's tracks by key. Returns rows removed. DB-cache op.
    async fn delete_tracks(&self, keys: &[String]) -> Result<u64, SourceError> {
        self.db()
            .delete_tracks(self.source(), keys)
            .await
            .map_err(SourceError::from)
    }

    /// Delete an album and its tracks for this source. DB-cache op.
    async fn delete_album(&self, album_id: &str) -> Result<(), SourceError> {
        self.db()
            .delete_album(self.source(), album_id)
            .await
            .map_err(SourceError::from)
    }

    /// Upsert this source's albums into the DB cache. DB-cache op.
    async fn upsert_albums(&self, albums: &[reader::Album]) -> Result<(), SourceError> {
        self.db()
            .upsert_albums(self.source(), albums)
            .await
            .map_err(SourceError::from)
    }

    /// Drop this source's rows absent from the kept sets — the post-sync reconcile
    /// against the remote's current contents. DB-cache op.
    async fn prune(
        &self,
        keep_track_keys: &[String],
        keep_album_ids: &[String],
    ) -> Result<(), SourceError> {
        self.db()
            .prune_source(self.source(), keep_track_keys, keep_album_ids)
            .await
            .map_err(SourceError::from)
    }

    /// Name the artist this source issued `id` for as its own record does; answers whether the row changed. DB op.
    async fn name_artist(&self, id: &str, name: &str) -> Result<bool, SourceError> {
        self.db()
            .name_artist(self.source(), id, name)
            .await
            .map_err(SourceError::from)
    }

    /// Name every artist of this source's own listing that carries an id; answers how many rows changed. DB op.
    async fn name_listed_artists(
        &self,
        listed: &[(reader::ArtistCredit, Option<String>)],
    ) -> Result<usize, SourceError> {
        let named: Vec<(String, String)> = listed
            .iter()
            .filter_map(|(artist, _)| Some((artist.id.clone()?, artist.name.clone())))
            .collect();
        if named.is_empty() {
            return Ok(0);
        }
        self.db()
            .name_artists(self.source(), &named)
            .await
            .map_err(SourceError::from)
    }

    /// Record (or clear) the cached image for one of this source's artists, by its key. DB-cache op.
    async fn set_artist_image(
        &self,
        artist_key: &str,
        kind: &str,
        image_ref: Option<&str>,
    ) -> Result<(), SourceError> {
        self.db()
            .set_artist_image(self.source(), artist_key, kind, image_ref)
            .await
            .map_err(SourceError::from)
    }

    /// Mark a track downloaded at `path` (or clear it with `None`). DB-cache op.
    async fn set_offline_track(&self, id: &str, path: Option<&str>) -> Result<(), SourceError> {
        self.db()
            .set_offline_track(id, path)
            .await
            .map_err(SourceError::from)
    }

    /// Create a playlist folder (local organisation). DB-cache op.
    async fn create_folder(&self, id: &str, name: &str) -> Result<(), SourceError> {
        self.db()
            .create_folder(id, name)
            .await
            .map_err(SourceError::from)
    }

    /// Rename a playlist folder. DB-cache op.
    async fn rename_folder(&self, id: &str, name: &str) -> Result<(), SourceError> {
        self.db()
            .rename_folder(id, name)
            .await
            .map_err(SourceError::from)
    }

    /// Delete a playlist folder (its playlists become unfiled). DB-cache op.
    async fn delete_folder(&self, id: &str) -> Result<(), SourceError> {
        self.db().delete_folder(id).await.map_err(SourceError::from)
    }

    /// Move a playlist into a folder (or out of one with `None`). DB-cache op.
    async fn set_playlist_folder(
        &self,
        playlist_ref: &str,
        folder_id: Option<&str>,
    ) -> Result<(), SourceError> {
        self.db()
            .set_playlist_folder(playlist_ref, folder_id)
            .await
            .map_err(SourceError::from)
    }

    /// Set (or clear) this source's album cover, marking it manual or not. DB-cache op.
    async fn update_album_cover(
        &self,
        album_id: &str,
        cover_path: Option<&str>,
        manual: bool,
    ) -> Result<(), SourceError> {
        self.db()
            .update_album_cover(self.source(), album_id, cover_path, manual)
            .await
            .map_err(SourceError::from)
    }

    /// Drop this source's favorites not seen since `epoch` (post-pull reconcile).
    async fn sweep_favorites(&self, epoch: i64) -> Result<(), SourceError> {
        self.db()
            .sweep_favorites(self.source().as_str(), epoch)
            .await
            .map_err(SourceError::from)
    }

    /// Replace this source's clean favorites with `refs`, keeping pending (dirty)
    /// local toggles. DB-cache op.
    async fn replace_favorites_clean(&self, refs: &[String]) -> Result<(), SourceError> {
        self.db()
            .replace_favorites_clean(self.source().as_str(), refs)
            .await
            .map_err(SourceError::from)
    }

    /// Increment a track's play count in this source. DB-cache op.
    async fn bump_listen_count(&self, track_key: &str) -> Result<(), SourceError> {
        self.db()
            .bump_listen_count(self.source(), track_key)
            .await
            .map_err(SourceError::from)
    }

    /// Record a play in this source's recently-played history. DB-cache op.
    async fn record_recent(&self, track_key: &str) -> Result<(), SourceError> {
        self.db()
            .push_recent(self.source(), track_key)
            .await
            .map_err(SourceError::from)
    }

    /// Write a metadata-cache row — e.g. a sync timestamp. DB-cache op.
    async fn set_meta(
        &self,
        cache_key: &str,
        kind: &str,
        payload: &str,
    ) -> Result<(), SourceError> {
        self.db()
            .meta_put(cache_key, kind, payload)
            .await
            .map_err(SourceError::from)
    }

    /// Stamp one page of this source's favorites (rank + epoch) during a
    /// paginated pull. DB-cache op.
    async fn upsert_favorites_page(
        &self,
        refs: &[String],
        start_rank: i64,
        epoch: i64,
    ) -> Result<(), SourceError> {
        self.db()
            .upsert_favorites_page(self.source().as_str(), refs, start_rank, epoch)
            .await
            .map_err(SourceError::from)
    }

    /// Streaming upsert of one page of a playlist's entries, stamped with `epoch`
    /// (mirrors [`upsert_favorites_page`]). Resolved against this source's
    /// partition — callers never spell out a `source`.
    async fn upsert_playlist_tracks_page(
        &self,
        playlist_id: &str,
        entries: &[reader::PlaylistEntry],
        start_position: i64,
        epoch: i64,
    ) -> Result<(), SourceError> {
        self.db()
            .upsert_playlist_tracks_page(self.source(), playlist_id, entries, start_position, epoch)
            .await
            .map_err(SourceError::from)
    }

    /// End-of-walk sweep for a playlist: drop entries not re-stamped with `epoch`.
    async fn sweep_playlist_tracks(
        &self,
        playlist_id: &str,
        epoch: i64,
    ) -> Result<(), SourceError> {
        self.db()
            .sweep_playlist_tracks(self.source(), playlist_id, epoch)
            .await
            .map_err(SourceError::from)
    }

    /// Upsert this source's playlist listing metadata (name/cover/image tag).
    /// DB-cache op.
    async fn upsert_playlist_meta(
        &self,
        pl_id: &str,
        name: &str,
        cover_path: Option<&str>,
        image_tag: Option<&str>,
    ) -> Result<(), SourceError> {
        self.db()
            .upsert_playlist_meta(self.source(), pl_id, name, cover_path, image_tag)
            .await
            .map_err(SourceError::from)
    }
}

/// Mirror a successful remote playlist-add into the DB cache (so the DB-reading
/// UI reflects it without waiting for a sync).
pub(super) async fn mirror_added(
    db: &Db,
    source: &Source,
    pid: &str,
    added: &[String],
) -> Result<(), SourceError> {
    if !added.is_empty() {
        db.add_playlist_tracks(source, pid, added).await?;
    }
    Ok(())
}

/// Mirror a remote playlist-create into the DB cache.
pub(super) async fn mirror_created(
    db: &Db,
    source: &Source,
    id: &str,
    name: &str,
    refs: &[String],
) -> Result<(), SourceError> {
    db.upsert_playlist_meta(source, id, name, None, None)
        .await?;
    let entries: Vec<reader::PlaylistEntry> = refs.iter().map(|key| key.as_str().into()).collect();
    db.set_playlist_tracks(source, id, &entries)
        .await
        .map_err(SourceError::from)
}

// ============================ Resolvers ================================

/// Build the per-remote source for `conn` — the ONE place service dispatch happens.
fn remote_source(db: Db, source: Source, conn: &ServerConn) -> Box<dyn MediaSource> {
    match conn.service {
        MusicService::Jellyfin => Box::new(JellyfinSource::new(db, source, conn)),
        MusicService::Subsonic | MusicService::Custom => {
            Box::new(SubsonicSource::new(db, source, conn))
        }
        MusicService::YtMusic => Box::new(YtSource::new(db, source, conn)),
        MusicService::SoundCloud => Box::new(SoundcloudSource::new(db, source, conn)),
        MusicService::AppleMusic => {
            crate::applemusic::widevine::fetch::prefetch();
            Box::new(AppleMusicSource::new(
                db,
                source,
                crate::applemusic::AppleMusicApi::new(
                    Some(conn.token.clone()),
                    &conn.apple_music_storefront,
                    &conn.apple_music_language,
                ),
            ))
        }
        MusicService::Spotify => Box::new(SpotifySource::new(db, source, conn)),
        MusicService::Nextcloud => Box::new(NextcloudSource::new(db, source, conn)),
        MusicService::Clippsly => Box::new(ClippslySource::new(db, source, conn)),
        MusicService::Smb => Box::new(SmbSource::new(db, source, conn)),
        MusicService::AudioCd => Box::new(audio_cd::AudioCdSource::new(db, source, &conn.url)),
    }
}

/// Shared backend handle. Reuse it across operations to retain the HTTP client.
pub type ActiveSource = std::sync::Arc<dyn MediaSource>;

/// The configured server's [`MediaSource`], or `None` when no usable creds
/// exist. Unlike [`active`] this ignores the active source — the reconciler
/// syncs the configured server even while a local page is open.
pub fn configured_server(db: Db, sources: &Sources) -> Option<Box<dyn MediaSource>> {
    let conn = ServerConn::resolve(sources)?;
    let source = Source::Server(sources.active_server_id().unwrap_or_default());
    Some(remote_source(db, source, &conn))
}

/// A local (filesystem) [`MediaSource`] using the supplied DB namespace.
pub fn local(db: Db, source: Source) -> Box<dyn MediaSource> {
    Box::new(LocalSource { db, source })
}

/// The single factory: a server without usable creds in `sources` gets the offline stand-in, and building one stands up an HTTP client, so hold the result.
pub fn resolve(db: Db, sources: &Sources, source: &Source) -> Box<dyn MediaSource> {
    match source {
        Source::LocalLibrary(_) => local(db, source.clone()),
        Source::Server(id) => match ServerConn::resolve(sources) {
            Some(conn) => remote_source(db, Source::Server(id.clone()), &conn),
            None => Box::new(OfflineServerSource {
                db,
                source: Source::Server(id.clone()),
            }),
        },
    }
}

/// The [`MediaSource`] for the app's active source.
pub fn active(db: Db, sources: &Sources) -> Box<dyn MediaSource> {
    resolve(db, sources, &sources.active)
}

/// Rows for an inserted disc, without another drive read.
pub fn audio_cd_snapshot(disc: crate::audio_cd::Disc) -> LibrarySnapshot {
    audio_cd::snapshot(disc)
}
