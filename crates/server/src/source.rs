//! The unified media-source facade (issue #347, Phase 2).
//!
//! [`MediaSource`] is the SINGLE per-source backend: one impl per source kind
//! (`LocalSource`, the per-remote `JellyfinSource`/`SubsonicSource`/`YtSource`,
//! and a creds-less `OfflineServerSource`). There is no separate remote-client
//! trait — a per-remote impl wraps its raw HTTP client directly, so dispatch
//! happens once in [`resolve`] and adding a service is one new impl.
//!
//! The method split is three-way:
//! * Uniform source-scoped DB ops (favorites, the cache mutators) are default
//!   methods keyed on [`source`](MediaSource::source) — written once, inherited.
//! * The ops every source implements but differently (`add_to_playlist`,
//!   `create_playlist`, `remove_from_playlist`, `resolve_stream`, `validate`,
//!   `fetch_favorites`, `push_favorite`) are required.
//! * Optional, capability-gated ops (`reorder_playlist`, and future
//!   radio/discover/download) default to [`SourceError::Unsupported`]; only a
//!   source that declares the matching [`Capabilities`] flag overrides them, and
//!   the UI gates the affordance on that flag so the default is never reached.
//!
//! Each impl declares its own [`capabilities`](MediaSource::capabilities) — the
//! UI reads them off the resolved source instead of branching on `is_server()`
//! or `match service`. Adding a backend is one impl + its caps literal.
//!
//! Reactivity stays out (this crate is Dioxus-free): callers bump generations /
//! nudge the sync task after a successful op.

use async_trait::async_trait;

use config::{AppConfig, MusicService, Source};
use db::Db;

use crate::server_ops::ServerConn;

pub mod capabilities;
mod jellyfin;
mod local;
mod offline;
mod soundcloud;
mod subsonic;
mod types;
mod youtube_music;
use jellyfin::JellyfinSource;
use local::LocalSource;
use offline::OfflineServerSource;
use soundcloud::SoundcloudSource;
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

    /// Remove one track from a playlist. The per-service identity differs (YT:
    /// video id, Jellyfin: entry id, Subsonic: position), so the whole track +
    /// its current position are passed and each impl takes what it needs; the
    /// DB cache is kept in sync.
    async fn remove_from_playlist(
        &self,
        playlist_id: &str,
        track: &reader::Track,
        position: usize,
    ) -> Result<(), SourceError>;

    /// Resolve a playable stream for one item id (local = a file path, server =
    /// the remote's URL / deciphered stream).
    async fn resolve_stream(&self, item_id: &str) -> Result<StreamInfo, SourceError>;

    /// Check stored creds against the source (local is always [`Valid`](AuthOutcome::Valid)).
    async fn validate(&self) -> AuthOutcome;

    /// All favorited item ids on the remote (YT pages internally). Local has no
    /// remote set, so it returns empty.
    async fn fetch_favorites(&self) -> Result<Vec<String>, SourceError>;

    /// Push one favorite to the remote (the reconciler's flush). A no-op for
    /// local (its favorites are already the DB rows).
    async fn push_favorite(&self, item_id: &str, on: bool) -> Result<(), SourceError>;

    // --- capability-gated ops (default = unsupported) -----------------------

    /// Persist a playlist reorder: `ordered_refs` is the full new membership;
    /// `moved`/`new_index` identify the one entry that changed position. Only
    /// sources whose [`Capabilities::playlists`] is [`PlaylistOps::Reorder`]
    /// override this; the rest inherit the unsupported default.
    async fn reorder_playlist(
        &self,
        _playlist_id: &str,
        _ordered_refs: &[String],
        _moved: &reader::Track,
        _new_index: usize,
    ) -> Result<(), SourceError> {
        Err(SourceError::unsupported("playlist reorder"))
    }

    /// Start a radio/mix seeded from a track, returning the generated queue. Only
    /// sources whose [`Capabilities::radio`] is set override this; the rest
    /// inherit the unsupported default.
    async fn start_radio(&self, _seed_ref: &str) -> Result<Vec<reader::Track>, SourceError> {
        Err(SourceError::unsupported("radio"))
    }

    /// The track's canonical public web URL, when this source has shareable web
    /// pages (e.g. a YouTube Music watch link). `None` otherwise — callers fall
    /// back to a metadata lookup (MusicBrainz). Sync: it's a pure id→URL mapping.
    fn web_url(&self, _track: &reader::Track) -> Option<String> {
        None
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
        Ok(search_filter(&q, tracks, albums))
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

    /// The tracks of a remote album / browse id (discover surfaces). Default
    /// unsupported; only catalog remotes (YT) override.
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

    /// Resolve an opened album reference — a raw browse id, a `ytmusic:album:MPRE…`
    /// (search rows with an album link), or a synthesized `ytmusic:album:<hash>`
    /// (search rows without one) — to its full remote album. `None` when it can't
    /// be resolved or has no tracks. The id→browse-id→album dance lives here so the
    /// UI never reaches into the per-service catalog. Default unsupported; only
    /// catalog remotes (YT) override.
    async fn fetch_album_by_ref(&self, _id: &str) -> Result<Option<RemoteAlbum>, SourceError> {
        Err(SourceError::unsupported("album by ref"))
    }

    /// Resolve a saved album's title + artist to its full remote album (header +
    /// every track) for the YT-Music-style album page — the local library stores
    /// YT albums by hash with no browse id, so the page needs the remote listing.
    /// `None` when it can't be resolved or has no tracks. Default unsupported; only
    /// catalog remotes (YT) override.
    async fn fetch_album_by_meta(
        &self,
        _title: &str,
        _artist: &str,
    ) -> Result<Option<RemoteAlbum>, SourceError> {
        Err(SourceError::unsupported("album by meta"))
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

    /// Resolve an artist name to a remote channel id (discover artist links).
    /// Default unsupported; only catalog remotes (YT) override.
    async fn resolve_artist_channel_id(&self, _query: &str) -> Result<Option<String>, SourceError> {
        Err(SourceError::unsupported("artist channel"))
    }

    /// Resolve a saved album's title + artist to a remote album browse id, so
    /// the album page can fetch the album's full track list (the local library
    /// stores YT albums by hash, with no browse id). Default unsupported; only
    /// catalog remotes (YT) override.
    async fn resolve_album_browse_id(
        &self,
        _album: &str,
        _artist: &str,
    ) -> Result<Option<String>, SourceError> {
        Err(SourceError::unsupported("album browse id"))
    }

    /// A remote artist profile (banner, top songs, albums, related) by channel
    /// id. Default unsupported; only catalog remotes (YT) override.
    async fn fetch_artist(
        &self,
        _channel_id: &str,
    ) -> Result<crate::ytmusic::discover::YtArtist, SourceError> {
        Err(SourceError::unsupported("artist profile"))
    }

    // --- remote reads (default = nothing; servers override) ----------------

    /// Pull the source's entire remote library (albums, tracks, artist images),
    /// transformed into model types. Default empty — only library remotes
    /// (Jellyfin/Subsonic) override; the caller persists + prunes the result.
    async fn fetch_library(&self) -> Result<LibrarySnapshot, SourceError> {
        Ok(LibrarySnapshot::default())
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
        })
    }

    /// Fetch the source's playlists (id, name, image tag) from the remote — the
    /// listing only; entries come from [`fetch_playlist_entries`]. Default empty.
    async fn fetch_playlists(&self) -> Result<Vec<PlaylistMeta>, SourceError> {
        Ok(Vec::new())
    }

    /// Fetch artist → image-URL pairs from the remote (for the "artist photo"
    /// view). Default empty (local reads them from the DB; YT has none).
    async fn fetch_artist_images(&self) -> Result<Vec<(String, String)>, SourceError> {
        Ok(Vec::new())
    }

    /// Resolve a single artist's photo URL by name. Default None; the catalog
    /// remote (YT) implements it so the Artists grid can show real YT photos.
    async fn fetch_artist_image(&self, _name: &str) -> Result<Option<String>, SourceError> {
        Ok(None)
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

    // --- uniform ops (default): a plain source-scoped DB read/write ---------

    /// One album's tracks (disc/track-ordered), read from this source's library
    /// cache. Uniform across sources — the UI goes through the source rather than
    /// touching the DB directly.
    async fn album_tracks(&self, album_id: &str) -> Result<Vec<reader::Track>, SourceError> {
        self.db()
            .album_tracks(self.source(), album_id)
            .await
            .map_err(SourceError::from)
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
        // Cache the track so the favorites view can resolve the ref.
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
        refs: &[String],
    ) -> Result<(), SourceError> {
        self.db()
            .set_playlist_tracks(self.source(), playlist_id, refs)
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

    /// Record (or clear) the cached image for an artist. DB-cache op.
    async fn set_artist_image(
        &self,
        artist_norm: &str,
        kind: &str,
        image_ref: Option<&str>,
    ) -> Result<(), SourceError> {
        self.db()
            .set_artist_image(artist_norm, kind, image_ref)
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

    /// Increment a track's play count, keyed by its uid. DB-cache op.
    async fn bump_listen_count(&self, track_uid: &str) -> Result<(), SourceError> {
        self.db()
            .bump_listen_count(track_uid)
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
        refs: &[String],
        start_position: i64,
        epoch: i64,
    ) -> Result<(), SourceError> {
        self.db()
            .upsert_playlist_tracks_page(self.source(), playlist_id, refs, start_position, epoch)
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

/// Encode a cover URL into the `urlhex_…` tag the Subsonic cover seam decodes
/// back (the synthetic album/track cover reference for Subsonic/Custom).
pub(super) fn encode_cover_url_tag(url: &str) -> String {
    let mut hex = String::with_capacity(url.len() * 2);
    for b in url.as_bytes() {
        hex.push_str(&format!("{b:02x}"));
    }
    format!("urlhex_{hex}")
}

/// Filter a library corpus by a lowercased `query` — the shared search behavior
/// for corpus-backed sources (local, Jellyfin, Subsonic). Matches tracks on
/// title/artist/album/genre (≤100) and albums on title/artist/genre, deduped by
/// title (≤30). Covers are resolved by the caller via the cover seam.
fn search_filter(
    query: &str,
    tracks: Vec<reader::Track>,
    albums: Vec<reader::Album>,
) -> (Vec<reader::Track>, Vec<reader::Album>) {
    let album_genre: std::collections::HashMap<&String, &str> =
        albums.iter().map(|a| (&a.id, a.genre.as_str())).collect();

    let result_tracks: Vec<reader::Track> = tracks
        .iter()
        .filter(|t| {
            t.title.to_lowercase().contains(query)
                || t.artist.to_lowercase().contains(query)
                || t.album.to_lowercase().contains(query)
                || album_genre
                    .get(&t.album_id)
                    .map(|g| g.to_lowercase().contains(query))
                    .unwrap_or(false)
        })
        .take(100)
        .cloned()
        .collect();

    let mut seen = std::collections::HashSet::new();
    let result_albums: Vec<reader::Album> = albums
        .iter()
        .filter(|a| {
            (a.title.to_lowercase().contains(query)
                || a.artist.to_lowercase().contains(query)
                || a.genre.to_lowercase().contains(query))
                && seen.insert(a.title.trim().to_lowercase())
        })
        .take(30)
        .cloned()
        .collect();

    (result_tracks, result_albums)
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
    db.set_playlist_tracks(source, id, refs)
        .await
        .map_err(SourceError::from)
}

// ============================ Resolvers ================================

/// The server id of the active source — its own id, falling back to the
/// configured server's id (matches the legacy single-server config).
fn active_server_id(config: &AppConfig) -> Option<String> {
    config
        .active_source
        .server_id()
        .map(String::from)
        .or_else(|| config.server.as_ref().and_then(|s| s.id.clone()))
}

/// Build the per-remote source for `conn` — the ONE place service dispatch happens.
fn remote_source(db: Db, source: Source, conn: &ServerConn) -> Box<dyn MediaSource> {
    match conn.service {
        MusicService::Jellyfin => Box::new(JellyfinSource::new(db, source, conn)),
        MusicService::Subsonic | MusicService::Custom => {
            Box::new(SubsonicSource::new(db, source, conn))
        }
        MusicService::YtMusic => Box::new(YtSource::new(db, source, conn)),
        MusicService::SoundCloud => Box::new(SoundcloudSource::new(db, source, conn)),
    }
}

/// The active [`MediaSource`], shared and reference-counted. Held once in a
/// `Signal<ActiveSource>` (context) and swapped only on a source-switch or cred
/// rotation, so call sites read the cached handle instead of rebuilding — and
/// for a server, re-standing-up an HTTP client — on every operation.
pub type ActiveSource = std::sync::Arc<dyn MediaSource>;

/// The configured server's [`MediaSource`], or `None` when no usable creds
/// exist. Unlike [`active`] this ignores the active source — the reconciler
/// syncs the configured server even while a local page is open.
pub fn configured_server(db: Db, config: &AppConfig) -> Option<Box<dyn MediaSource>> {
    let conn = ServerConn::resolve(config)?;
    let source = Source::Server(active_server_id(config).unwrap_or_default());
    Some(remote_source(db, source, &conn))
}

/// The local (filesystem) [`MediaSource`] — for the statically-local pages,
/// which never act on a server and so need no config.
pub fn local(db: Db) -> Box<dyn MediaSource> {
    Box::new(LocalSource {
        db,
        source: Source::Local,
    })
}

/// The [`MediaSource`] backing a given [`Source`] key — the single factory.
/// Local needs no creds; a server resolves its creds from `config` (falling back
/// to the offline stand-in when none are usable, so favorites still queue).
/// Building a server source stands up an HTTP client, so resolve once and hold
/// the result (the cached [`ActiveSource`]) rather than calling per render.
pub fn resolve(db: Db, config: &AppConfig, source: &Source) -> Box<dyn MediaSource> {
    match source {
        Source::Local => local(db),
        Source::Server(id) => match ServerConn::resolve(config) {
            Some(conn) => remote_source(db, Source::Server(id.clone()), &conn),
            None => Box::new(OfflineServerSource {
                db,
                source: Source::Server(id.clone()),
            }),
        },
    }
}

/// The [`MediaSource`] for the app's active source.
pub fn active(db: Db, config: &AppConfig) -> Box<dyn MediaSource> {
    resolve(db, config, &config.active_source)
}
