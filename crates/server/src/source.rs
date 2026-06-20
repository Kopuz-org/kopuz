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

use std::fmt;

use async_trait::async_trait;

use config::{AppConfig, MusicService, Source};
use db::Db;

use crate::jellyfin::JellyfinClient;
use crate::server_ops::ServerConn;
use crate::subsonic::SubsonicClient;
use crate::ytmusic::YouTubeMusicClient;
use crate::ytmusic::player::AudioFormat;

/// Why a [`MediaSource`] operation failed, classified so the UI can react
/// differently (toast vs re-auth prompt vs "not supported") instead of pattern-
/// matching an opaque string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceError {
    /// This source doesn't offer the operation (capability-gated). Carries the
    /// op name; unreachable when the UI honours [`Capabilities`].
    Unsupported(&'static str),
    /// No usable connection to the remote (offline / creds-less stand-in).
    Connectivity,
    /// The remote rejected the credentials — re-auth needed.
    Auth,
    /// The caller passed something the operation can't act on (e.g. a track
    /// missing the id a remote needs).
    InvalidInput(String),
    /// An underlying client/DB error, kept as its message.
    Backend(String),
}

impl SourceError {
    /// Construct an [`Unsupported`](SourceError::Unsupported) for `op`.
    pub fn unsupported(op: &'static str) -> Self {
        SourceError::Unsupported(op)
    }
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceError::Unsupported(op) => write!(f, "this source doesn't support {op}"),
            SourceError::Connectivity => f.write_str("the server has no active connection"),
            SourceError::Auth => {
                f.write_str("this source isn't signed in — open Settings to re-sign in")
            }
            SourceError::InvalidInput(m) | SourceError::Backend(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for SourceError {}

impl From<String> for SourceError {
    fn from(m: String) -> Self {
        SourceError::Backend(m)
    }
}

impl From<db::DbError> for SourceError {
    fn from(e: db::DbError) -> Self {
        SourceError::Backend(e.to_string())
    }
}

/// What a source can do with playlists — a genuine three-way split (local and
/// most servers can reorder; YT Music can add/remove but not reorder; an
/// offline/creds-less server can do neither). `Ord` so the UI can gate with
/// `caps.playlists >= PlaylistOps::AddRemove`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PlaylistOps {
    /// No playlist mutation (no live connection).
    None,
    /// Create / add / remove, but no reordering.
    AddRemove,
    /// Full editing, including reordering entries.
    Reorder,
}

/// How a source's artist view behaves — the artists tab routes on this instead
/// of hardcoding a service. (`Unsupported` isn't modelled yet: every source has
/// some artist view, and an unconstructed variant would be dead code.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtistView {
    /// A library-driven page: a grid of artists → their tracks. Local, Jellyfin,
    /// Subsonic — and a creds-less/offline server (cached artists).
    Library,
    /// A rich remote profile (banner, top songs, albums, related) when an artist
    /// is selected; the library grid otherwise. YT Music.
    Remote,
}

/// How favorites are fetched. `Instant` returns the whole set in one call
/// ([`fetch_favorites`]); `Paginated` walks pages ([`fetch_favorites_page`]) so
/// the UI can stream rows + show progress (YT's 800-song liked library).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FavoritesSync {
    Instant,
    Paginated,
}

/// One page of paginated favorites: the tracks plus the cursor for the next page
/// (`None` once exhausted). Cross-page dedup is the caller's job.
pub struct FavoritesPage {
    pub tracks: Vec<reader::Track>,
    pub next: Option<String>,
}

/// One page of a playlist's entries: the tracks plus the cursor for the next page
/// (`None` once exhausted). Cross-page dedup is the caller's job. Sources that
/// can't page return everything in a single page (the default impl does this).
pub struct PlaylistPage {
    pub tracks: Vec<reader::Track>,
    pub next: Option<String>,
}

/// A full remote library pull: albums, tracks, and artist `(name, image_url)`
/// pairs, already transformed into the generic model types (each source applies
/// its own id-prefix / cover encoding). The caller persists + prunes; keeping the
/// write side out of the source preserves the chunked upsert/bump streaming.
#[derive(Default)]
pub struct LibrarySnapshot {
    pub albums: Vec<reader::Album>,
    pub tracks: Vec<reader::Track>,
    pub artist_images: Vec<(String, String)>,
}

/// What a source supports, declared by each impl and read off the resolved
/// source. Source-agnostic pages gate their few divergent affordances on these
/// flags rather than on the source kind or service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// Edit a track's tags (writes to the file) — local only.
    pub edit_tags: bool,
    /// Delete a track from disk — local only.
    pub delete_from_disk: bool,
    /// Scan/import filesystem folders — local only.
    pub scan_folders: bool,
    /// Organise playlists into folders (local) vs a flat remote list (servers).
    pub folders: bool,
    /// Manual refresh / background reconcile against a remote — servers.
    pub sync: bool,
    /// Download tracks for offline — servers.
    pub downloads: bool,
    /// A discover/recommendations surface — YT (and future remotes).
    pub discover: bool,
    /// Start a radio/mix from a seed track — YT.
    pub radio: bool,
    /// Playlist mutation level.
    pub playlists: PlaylistOps,
    /// How the artists tab renders a selected artist.
    pub artist_view: ArtistView,
    /// How favorites sync — one shot vs paginated (streamed with progress).
    pub favorites_sync: FavoritesSync,
}

/// What credential validation concluded. `Unreachable` ≠ `Expired`: a network
/// blip must not reprompt sign-in — only a real auth rejection does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthOutcome {
    Valid,
    Expired,
    Unreachable,
}

/// A resolved playable stream for one item. `format`/`user_agent`/`duration_secs`
/// /`bitrate` are populated only where the source provides them (YT's deciphered
/// stream carries all four; a plain progressive URL leaves them `None`).
pub struct StreamInfo {
    pub url: String,
    pub format: Option<(AudioFormat, bool)>,
    pub user_agent: Option<String>,
    pub duration_secs: Option<u64>,
    pub bitrate: Option<u32>,
    /// Total byte length of the stream, when the source reports it (YT does — the
    /// offline downloader uses it for progress). `None` for plain URL sources.
    pub content_length: Option<u64>,
}

/// A remote playlist's listing metadata (no entries). `image_tag` is the remote
/// cover reference (Jellyfin Primary tag / encoded YT thumbnail URL), `None` for
/// sources without one.
pub struct PlaylistMeta {
    pub id: String,
    pub name: String,
    pub image_tag: Option<String>,
}

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

    // --- uniform ops (default): a plain source-scoped DB write --------------

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
async fn mirror_added(
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
fn encode_cover_url_tag(url: &str) -> String {
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
async fn mirror_created(
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

// ============================ Local ====================================

/// The local (filesystem) source: every op is a DB write or a no-op.
struct LocalSource {
    db: Db,
    source: Source,
}

#[async_trait]
impl MediaSource for LocalSource {
    fn source(&self) -> &Source {
        &self.source
    }
    fn db(&self) -> &Db {
        &self.db
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            edit_tags: true,
            delete_from_disk: true,
            scan_folders: true,
            folders: true,
            sync: false,
            downloads: false,
            discover: false,
            radio: false,
            playlists: PlaylistOps::Reorder,
            artist_view: ArtistView::Library,
            favorites_sync: FavoritesSync::Instant,
        }
    }

    async fn add_to_playlist(
        &self,
        playlist_id: &str,
        item_refs: &[String],
    ) -> Result<Vec<String>, SourceError> {
        self.db
            .add_playlist_tracks(&self.source, playlist_id, item_refs)
            .await?;
        Ok(item_refs.to_vec())
    }

    async fn create_playlist(
        &self,
        name: &str,
        item_refs: &[String],
    ) -> Result<String, SourceError> {
        let id = uuid::Uuid::new_v4().to_string();
        mirror_created(&self.db, &self.source, &id, name, item_refs).await?;
        Ok(id)
    }

    async fn remove_from_playlist(
        &self,
        playlist_id: &str,
        track: &reader::Track,
        _position: usize,
    ) -> Result<(), SourceError> {
        let r = track.id.key().into_owned();
        self.db
            .remove_playlist_tracks(&self.source, playlist_id, &[r])
            .await
            .map_err(SourceError::from)
    }

    async fn reorder_playlist(
        &self,
        playlist_id: &str,
        ordered_refs: &[String],
        _moved: &reader::Track,
        _new_index: usize,
    ) -> Result<(), SourceError> {
        self.db
            .set_playlist_tracks(&self.source, playlist_id, ordered_refs)
            .await
            .map_err(SourceError::from)
    }

    async fn resolve_stream(&self, item_id: &str) -> Result<StreamInfo, SourceError> {
        Ok(StreamInfo {
            url: item_id.to_string(),
            format: None,
            user_agent: None,
            duration_secs: None,
            bitrate: None,
            content_length: None,
        })
    }

    async fn validate(&self) -> AuthOutcome {
        AuthOutcome::Valid
    }

    async fn fetch_favorites(&self) -> Result<Vec<String>, SourceError> {
        Ok(Vec::new())
    }

    async fn push_favorite(&self, _item_id: &str, _on: bool) -> Result<(), SourceError> {
        Ok(())
    }
}

// ===================== Server, no live connection ======================

/// A server whose creds are missing/unusable: favorites + cache mutators still
/// work (DB defaults), every remote op errors. Keeps anonymous likes queuing.
struct OfflineServerSource {
    db: Db,
    source: Source,
}

#[async_trait]
impl MediaSource for OfflineServerSource {
    fn source(&self) -> &Source {
        &self.source
    }
    fn db(&self) -> &Db {
        &self.db
    }

    fn capabilities(&self) -> Capabilities {
        // Creds-less / unreachable: no remote affordances. Favorites still queue
        // (a default DB op, not gated here).
        Capabilities {
            edit_tags: false,
            delete_from_disk: false,
            scan_folders: false,
            folders: false,
            sync: false,
            downloads: false,
            discover: false,
            radio: false,
            playlists: PlaylistOps::None,
            artist_view: ArtistView::Library,
            favorites_sync: FavoritesSync::Instant,
        }
    }

    async fn add_to_playlist(&self, _: &str, _: &[String]) -> Result<Vec<String>, SourceError> {
        Err(SourceError::Connectivity)
    }
    async fn create_playlist(&self, _: &str, _: &[String]) -> Result<String, SourceError> {
        Err(SourceError::Connectivity)
    }
    async fn remove_from_playlist(
        &self,
        _: &str,
        _: &reader::Track,
        _: usize,
    ) -> Result<(), SourceError> {
        Err(SourceError::Connectivity)
    }
    async fn resolve_stream(&self, _: &str) -> Result<StreamInfo, SourceError> {
        Err(SourceError::Auth)
    }
    async fn validate(&self) -> AuthOutcome {
        AuthOutcome::Unreachable
    }
    async fn fetch_favorites(&self) -> Result<Vec<String>, SourceError> {
        Err(SourceError::Connectivity)
    }
    async fn push_favorite(&self, _: &str, _: bool) -> Result<(), SourceError> {
        Err(SourceError::Connectivity)
    }
}

// ============================ Jellyfin =================================

struct JellyfinSource {
    db: Db,
    source: Source,
    client: JellyfinClient,
}

#[async_trait]
impl MediaSource for JellyfinSource {
    fn source(&self) -> &Source {
        &self.source
    }
    fn db(&self) -> &Db {
        &self.db
    }

    async fn keepalive(&self) -> Result<(), SourceError> {
        self.client.ping().await.map_err(SourceError::from)
    }

    async fn report_playback_start(&self, item_id: &str) -> Result<(), SourceError> {
        self.client
            .report_playback_start(item_id)
            .await
            .map_err(SourceError::from)
    }

    async fn report_playback_stopped(
        &self,
        item_id: &str,
        position_ticks: u64,
    ) -> Result<(), SourceError> {
        self.client
            .report_playback_stopped(item_id, position_ticks)
            .await
            .map_err(SourceError::from)
    }

    async fn report_playback_progress(
        &self,
        item_id: &str,
        position_ticks: u64,
        is_paused: bool,
    ) -> Result<(), SourceError> {
        self.client
            .report_playback_progress(item_id, position_ticks, is_paused)
            .await
            .map_err(SourceError::from)
    }

    async fn fetch_library(&self) -> Result<LibrarySnapshot, SourceError> {
        use std::path::PathBuf;
        let mut albums = Vec::new();
        let mut tracks = Vec::new();
        let mut artist_images = Vec::new();

        for lib in self.client.get_music_libraries().await? {
            let mut start = 0;
            let limit = 500;
            loop {
                let (page, _total) = self
                    .client
                    .get_albums_paginated(&lib.id, start, limit)
                    .await?;
                if page.is_empty() {
                    break;
                }
                let count = page.len();
                for a in page {
                    let image_tag = a
                        .image_tags
                        .as_ref()
                        .and_then(|t| t.get("Primary").cloned());
                    let cover_path = Some(PathBuf::from(match &image_tag {
                        Some(tag) => format!("jellyfin:{}:{}", a.id, tag),
                        None => format!("jellyfin:{}", a.id),
                    }));
                    albums.push(reader::Album {
                        id: format!("jellyfin:{}", a.id),
                        title: a.name,
                        artist: a
                            .album_artist
                            .or_else(|| a.artists.as_ref().map(|x| x.join(", ")))
                            .unwrap_or_default(),
                        genre: a.genres.as_ref().map(|g| g.join(", ")).unwrap_or_default(),
                        year: a
                            .production_year
                            .map(|y| u16::try_from(y).unwrap_or(u16::MAX))
                            .unwrap_or(0),
                        cover_path,
                        manual_cover: false,
                    });
                }
                start += count;
                if count < limit {
                    break;
                }
            }

            let mut start = 0;
            let limit = 500;
            loop {
                let items = self
                    .client
                    .get_music_library_items_paginated(&lib.id, start, limit)
                    .await?;
                if items.is_empty() {
                    break;
                }
                let count = items.len();
                for item in items {
                    let cover = item
                        .image_tags
                        .as_ref()
                        .and_then(|tags| tags.get("Primary").cloned());
                    let bitrate_u16 =
                        (item.bitrate.unwrap_or(0) / 1000).min(u16::MAX as u32) as u16;
                    tracks.push(reader::Track {
                        id: reader::models::TrackId::Server {
                            service: MusicService::Jellyfin,
                            item_id: item.id.clone(),
                        },
                        cover,
                        album_id: item
                            .album_id
                            .map(|id| format!("jellyfin:{}", id))
                            .unwrap_or_default(),
                        title: item.name,
                        artist: item
                            .album_artist
                            .clone()
                            .or_else(|| item.artists.as_ref().map(|a| a.join(", ")))
                            .unwrap_or_default(),
                        album: item.album.unwrap_or_default(),
                        duration: item.run_time_ticks.unwrap_or(0) / 10_000_000,
                        khz: item.sample_rate.unwrap_or(0),
                        bitrate: bitrate_u16,
                        track_number: item.index_number,
                        disc_number: item.parent_index_number,
                        musicbrainz_release_id: None,
                        musicbrainz_recording_id: None,
                        musicbrainz_track_id: None,
                        playlist_item_id: None,
                        artists: item
                            .artists
                            .unwrap_or_else(|| item.album_artist.into_iter().collect()),
                    });
                }
                start += count;
                if count < limit {
                    break;
                }
            }
        }

        if let Ok(artists) = self.client.get_artists().await {
            for artist in artists {
                if let Some(tags) = &artist.image_tags
                    && let Some(tag) = tags.get("Primary")
                {
                    let url = utils::jellyfin_image::jellyfin_image_url(
                        self.client.base_url(),
                        &artist.id,
                        Some(tag.as_str()),
                        self.client.token(),
                        512,
                        90,
                    );
                    artist_images.push((artist.name, url));
                }
            }
        }

        Ok(LibrarySnapshot {
            albums,
            tracks,
            artist_images,
        })
    }

    async fn set_playlist_cover(
        &self,
        playlist_id: &str,
        name: &str,
        image_path: &std::path::Path,
        image_tag: Option<&str>,
    ) -> Result<(), SourceError> {
        // Push the artwork to Jellyfin (best-effort), then record it locally.
        if let Ok(bytes) = std::fs::read(image_path) {
            let ct = match image_path
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_lowercase)
                .as_deref()
            {
                Some("png") => "image/png",
                _ => "image/jpeg",
            };
            let _ = self.client.set_playlist_image(playlist_id, bytes, ct).await;
        }
        let cover = image_path.to_string_lossy();
        self.db()
            .upsert_playlist_meta(self.source(), playlist_id, name, Some(&cover), image_tag)
            .await
            .map_err(SourceError::from)
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            edit_tags: false,
            delete_from_disk: false,
            scan_folders: false,
            folders: false,
            sync: true,
            downloads: true,
            discover: false,
            radio: false,
            playlists: PlaylistOps::Reorder,
            artist_view: ArtistView::Library,
            favorites_sync: FavoritesSync::Instant,
        }
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
        let entry_id = track
            .playlist_item_id
            .as_deref()
            .ok_or_else(|| SourceError::InvalidInput("track has no playlist-entry id".into()))?;
        self.client
            .remove_from_playlist(playlist_id, entry_id)
            .await?;
        self.db
            .remove_playlist_tracks(&self.source, playlist_id, &[track.id.key().into_owned()])
            .await
            .map_err(SourceError::from)
    }

    async fn resolve_stream(&self, item_id: &str) -> Result<StreamInfo, SourceError> {
        Ok(StreamInfo {
            url: self.client.stream_url(item_id),
            format: None,
            user_agent: None,
            duration_secs: None,
            bitrate: None,
            content_length: None,
        })
    }

    async fn validate(&self) -> AuthOutcome {
        match self.client.ping().await {
            Ok(()) => AuthOutcome::Valid,
            Err(e) if e.contains("401") || e.contains("403") => AuthOutcome::Expired,
            Err(_) => AuthOutcome::Unreachable,
        }
    }

    async fn fetch_favorites(&self) -> Result<Vec<String>, SourceError> {
        Ok(self
            .client
            .get_favorite_items()
            .await?
            .into_iter()
            .map(|i| i.id)
            .collect())
    }

    async fn push_favorite(&self, item_id: &str, on: bool) -> Result<(), SourceError> {
        if on {
            self.client.mark_favorite(item_id).await
        } else {
            self.client.unmark_favorite(item_id).await
        }
        .map_err(SourceError::from)
    }

    async fn reorder_playlist(
        &self,
        playlist_id: &str,
        ordered_refs: &[String],
        moved: &reader::Track,
        new_index: usize,
    ) -> Result<(), SourceError> {
        let entry_id = moved
            .playlist_item_id
            .as_deref()
            .ok_or_else(|| SourceError::InvalidInput("track has no playlist-entry id".into()))?;
        self.client
            .move_playlist_item(playlist_id, entry_id, new_index)
            .await?;
        self.db
            .set_playlist_tracks(&self.source, playlist_id, ordered_refs)
            .await
            .map_err(SourceError::from)
    }

    async fn fetch_playlists(&self) -> Result<Vec<PlaylistMeta>, SourceError> {
        Ok(self
            .client
            .get_playlists()
            .await?
            .into_iter()
            .map(|p| PlaylistMeta {
                id: p.id,
                name: p.name,
                image_tag: p
                    .image_tags
                    .as_ref()
                    .and_then(|tags| tags.get("Primary").cloned()),
            })
            .collect())
    }

    async fn fetch_playlist_entries(
        &self,
        playlist_id: &str,
    ) -> Result<Vec<reader::Track>, SourceError> {
        let items = self.client.get_playlist_items(playlist_id).await?;
        Ok(items
            .into_iter()
            .map(|item| {
                let duration_secs = item.run_time_ticks.unwrap_or(0) / 10_000_000;
                let cover = item
                    .image_tags
                    .as_ref()
                    .and_then(|tags| tags.get("Primary").cloned());
                let bitrate_kbps = item.bitrate.unwrap_or(0) / 1000;
                let artist_str = item
                    .album_artist
                    .clone()
                    .or_else(|| item.artists.as_ref().map(|a| a.join(", ")))
                    .unwrap_or_default();
                reader::models::Track {
                    id: reader::models::TrackId::Server {
                        service: MusicService::Jellyfin,
                        item_id: item.id.clone(),
                    },
                    cover,
                    album_id: item
                        .album_id
                        .map(|id| format!("jellyfin:{}", id))
                        .unwrap_or_default(),
                    title: item.name,
                    artist: artist_str,
                    album: item.album.unwrap_or_default(),
                    duration: duration_secs,
                    khz: item.sample_rate.unwrap_or(0),
                    bitrate: bitrate_kbps.min(u16::MAX as u32) as u16,
                    track_number: item.index_number,
                    disc_number: item.parent_index_number,
                    musicbrainz_release_id: None,
                    musicbrainz_recording_id: None,
                    musicbrainz_track_id: None,
                    playlist_item_id: item.playlist_item_id,
                    artists: item.artists.unwrap_or_default(),
                }
            })
            .collect())
    }

    async fn fetch_artist_images(&self) -> Result<Vec<(String, String)>, SourceError> {
        let artists = self.client.get_artists().await?;
        let mut out = Vec::new();
        for artist in artists {
            if let Some(tags) = &artist.image_tags
                && let Some(tag) = tags.get("Primary")
            {
                out.push((
                    artist.name.clone(),
                    utils::jellyfin_image::jellyfin_image_url(
                        self.client.base_url(),
                        &artist.id,
                        Some(tag.as_str()),
                        self.client.token(),
                        512,
                        90,
                    ),
                ));
            }
        }
        Ok(out)
    }
}

// ============================ Subsonic =================================

struct SubsonicSource {
    db: Db,
    source: Source,
    client: SubsonicClient,
    /// Subsonic or Custom — both use this impl; the typed track id needs the
    /// exact one so its uid prefix round-trips.
    service: MusicService,
}

#[async_trait]
impl MediaSource for SubsonicSource {
    fn source(&self) -> &Source {
        &self.source
    }
    fn db(&self) -> &Db {
        &self.db
    }

    async fn scrobble_now_playing(&self, item_id: &str) -> Result<(), SourceError> {
        self.client
            .scrobble_now_playing(item_id)
            .await
            .map_err(SourceError::from)
    }

    async fn scrobble(&self, item_id: &str) -> Result<(), SourceError> {
        self.client
            .scrobble(item_id)
            .await
            .map_err(SourceError::from)
    }

    async fn fetch_library(&self) -> Result<LibrarySnapshot, SourceError> {
        use std::path::PathBuf;
        let prefix = match self.service {
            MusicService::Custom => "custom",
            _ => "subsonic",
        };
        let mut albums = Vec::new();
        let mut tracks = Vec::new();
        let mut artist_images = Vec::new();
        let mut seen = std::collections::HashSet::new();

        if let Ok(artists) = self.client.get_artists().await {
            for artist in artists {
                if let Some(cover_art_id) = &artist.cover_art
                    && let Ok(url) = self.client.cover_art_url(cover_art_id, Some(512))
                {
                    artist_images.push((artist.name, url));
                }
            }
        }

        let mut offset = 0;
        let batch = 250;
        loop {
            let page = self.client.get_album_list(offset, batch).await?;
            if page.is_empty() {
                break;
            }
            let count = page.len();
            for album in page {
                let album_cover_tag = album
                    .cover_art
                    .as_ref()
                    .and_then(|c| self.client.cover_art_url(c, Some(512)).ok())
                    .map(|url| encode_cover_url_tag(&url));
                let album_id_prefixed = match &album_cover_tag {
                    Some(tag) => format!("{}:{}:{}", prefix, album.id, tag),
                    None => format!("{}:{}:none", prefix, album.id),
                };
                let album_name = album.name.clone();
                let album_artist = album.artist.clone().unwrap_or_default();
                albums.push(reader::Album {
                    id: album_id_prefixed.clone(),
                    title: album_name.clone(),
                    artist: album_artist.clone(),
                    genre: album.genre.clone().unwrap_or_default(),
                    year: album.year.unwrap_or(0),
                    cover_path: Some(PathBuf::from(album_id_prefixed.clone())),
                    manual_cover: false,
                });

                let songs = self.client.get_album_songs(&album.id).await.map_err(|e| {
                    SourceError::Backend(format!(
                        "failed to fetch songs for album {}: {e}",
                        album.id
                    ))
                })?;
                for song in songs {
                    if !seen.insert(song.id.clone()) {
                        continue;
                    }
                    let bitrate_u16 = song.bit_rate.unwrap_or(0).min(u16::MAX as u32) as u16;
                    let song_cover_tag = song
                        .cover_art
                        .as_ref()
                        .and_then(|c| self.client.cover_art_url(c, Some(512)).ok())
                        .map(|url| encode_cover_url_tag(&url));
                    tracks.push(reader::Track {
                        id: reader::models::TrackId::Server {
                            service: self.service,
                            item_id: song.id.clone(),
                        },
                        cover: Some(song_cover_tag.unwrap_or_else(|| "none".to_string())),
                        album_id: album_id_prefixed.clone(),
                        title: song.title,
                        artist: song.artist.clone().unwrap_or_else(|| album_artist.clone()),
                        album: song.album.unwrap_or_else(|| album_name.clone()),
                        duration: song.duration.unwrap_or(0),
                        khz: song.sampling_rate.unwrap_or(0),
                        bitrate: bitrate_u16,
                        track_number: song.track,
                        disc_number: song.disc_number,
                        musicbrainz_release_id: None,
                        musicbrainz_recording_id: None,
                        musicbrainz_track_id: None,
                        playlist_item_id: None,
                        artists: vec![song.artist.unwrap_or_else(|| album_artist.clone())],
                    });
                }
            }
            offset += count;
            if count < batch {
                break;
            }
        }

        Ok(LibrarySnapshot {
            albums,
            tracks,
            artist_images,
        })
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            edit_tags: false,
            delete_from_disk: false,
            scan_folders: false,
            folders: false,
            sync: true,
            downloads: true,
            discover: false,
            radio: false,
            playlists: PlaylistOps::Reorder,
            artist_view: ArtistView::Library,
            favorites_sync: FavoritesSync::Instant,
        }
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
        position: usize,
    ) -> Result<(), SourceError> {
        self.client
            .remove_from_playlist(playlist_id, position)
            .await?;
        self.db
            .remove_playlist_tracks(&self.source, playlist_id, &[track.id.key().into_owned()])
            .await
            .map_err(SourceError::from)
    }

    async fn resolve_stream(&self, item_id: &str) -> Result<StreamInfo, SourceError> {
        Ok(StreamInfo {
            url: self.client.stream_url(item_id)?,
            format: None,
            user_agent: None,
            duration_secs: None,
            bitrate: None,
            content_length: None,
        })
    }

    async fn validate(&self) -> AuthOutcome {
        match self.client.ping().await {
            Ok(()) => AuthOutcome::Valid,
            Err(e)
                if e.contains("Wrong username")
                    || e.contains("not authorized")
                    || e.contains("code 40") =>
            {
                AuthOutcome::Expired
            }
            Err(_) => AuthOutcome::Unreachable,
        }
    }

    async fn fetch_favorites(&self) -> Result<Vec<String>, SourceError> {
        self.client
            .get_starred_song_ids()
            .await
            .map_err(SourceError::from)
    }

    async fn push_favorite(&self, item_id: &str, on: bool) -> Result<(), SourceError> {
        if on {
            self.client.star(item_id).await
        } else {
            self.client.unstar(item_id).await
        }
        .map_err(SourceError::from)
    }

    async fn reorder_playlist(
        &self,
        playlist_id: &str,
        ordered_refs: &[String],
        _moved: &reader::Track,
        _new_index: usize,
    ) -> Result<(), SourceError> {
        let ids: Vec<&str> = ordered_refs.iter().map(String::as_str).collect();
        self.client
            .reorder_playlist(playlist_id, &ids, ids.len())
            .await?;
        self.db
            .set_playlist_tracks(&self.source, playlist_id, ordered_refs)
            .await
            .map_err(SourceError::from)
    }

    async fn fetch_playlists(&self) -> Result<Vec<PlaylistMeta>, SourceError> {
        Ok(self
            .client
            .get_playlists()
            .await?
            .into_iter()
            .map(|p| PlaylistMeta {
                id: p.id,
                name: p.name,
                image_tag: None,
            })
            .collect())
    }

    async fn fetch_playlist_entries(
        &self,
        playlist_id: &str,
    ) -> Result<Vec<reader::Track>, SourceError> {
        let items = self.client.get_playlist_entries(playlist_id).await?;
        Ok(items
            .into_iter()
            .map(|item| {
                // Encode the cover URL as the `urlhex_` tag the cover resolver
                // understands; album_id carries it (or `:none`) like subsonic_sync.
                let cover_tag = item
                    .cover_art
                    .as_ref()
                    .and_then(|id| self.client.cover_art_url(id, Some(512)).ok())
                    .map(|url| format!("urlhex_{}", hex::encode(url.as_bytes())));
                let album_id = item
                    .album_id
                    .as_ref()
                    .map(|id| match &cover_tag {
                        Some(tag) => format!("jellyfin:{}:{}", id, tag),
                        None => format!("jellyfin:{}:none", id),
                    })
                    .unwrap_or_else(|| format!("jellyfin:{}:none", item.id));
                let artist = item.artist.clone().unwrap_or_default();
                reader::models::Track {
                    id: reader::models::TrackId::Server {
                        service: self.service,
                        item_id: item.id.clone(),
                    },
                    cover: Some(cover_tag.unwrap_or_else(|| "none".to_string())),
                    album_id,
                    title: item.title,
                    artist: artist.clone(),
                    album: item.album.unwrap_or_default(),
                    duration: item.duration.unwrap_or(0),
                    khz: item.sampling_rate.unwrap_or(0),
                    bitrate: item.bit_rate.unwrap_or(0).min(u16::MAX as u32) as u16,
                    track_number: item.track,
                    disc_number: item.disc_number,
                    musicbrainz_release_id: None,
                    musicbrainz_recording_id: None,
                    musicbrainz_track_id: None,
                    playlist_item_id: None,
                    artists: vec![artist],
                }
            })
            .collect())
    }

    async fn fetch_artist_images(&self) -> Result<Vec<(String, String)>, SourceError> {
        let artists = self.client.get_artists().await?;
        let mut out = Vec::new();
        for artist in artists {
            if let Some(cover_art_id) = &artist.cover_art
                && let Ok(url) = self.client.cover_art_url(cover_art_id, Some(512))
            {
                out.push((artist.name.clone(), url));
            }
        }
        Ok(out)
    }
}

// ========================= YouTube Music ==============================

struct YtSource {
    db: Db,
    source: Source,
    client: YouTubeMusicClient,
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

    fn web_url(&self, track: &reader::Track) -> Option<String> {
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

    async fn fetch_artist(
        &self,
        channel_id: &str,
    ) -> Result<crate::ytmusic::discover::YtArtist, SourceError> {
        self.client
            .fetch_artist(channel_id)
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
        MusicService::Jellyfin => Box::new(JellyfinSource {
            db,
            source,
            client: JellyfinClient::new(
                &conn.url,
                Some(&conn.token),
                &conn.device_id,
                Some(&conn.user_id),
            ),
        }),
        MusicService::Subsonic | MusicService::Custom => Box::new(SubsonicSource {
            db,
            source,
            client: SubsonicClient::new(&conn.url, &conn.user_id, &conn.token),
            service: conn.service,
        }),
        MusicService::YtMusic => Box::new(YtSource {
            db,
            source,
            client: YouTubeMusicClient::with_cookies(conn.token.clone()),
        }),
        MusicService::SoundCloud => Box::new(SoundcloudSource {
            db,
            source,
            token: (!conn.token.is_empty()).then(|| conn.token.clone()),
        }),
    }
}

/// The active [`MediaSource`], shared and reference-counted. Held once in a
/// `Signal<ActiveSource>` (context) and swapped only on a source-switch or cred
/// rotation, so call sites read the cached handle instead of rebuilding — and
/// for a server, re-standing-up an HTTP client — on every operation.
pub type ActiveSource = std::sync::Arc<dyn MediaSource>;

/// Ergonomic, track-centric wrappers over the source's DB-backed favorite truth,
/// so call sites read `track.is_favorite(&source).await` instead of pulling the
/// key out by hand. Defined here (not on `reader::Track`) because the truth lives
/// on [`MediaSource`], and `reader` can't depend on `server`.
#[async_trait]
pub trait TrackFavorite {
    /// Whether this track is currently favorited for `source` (an empty key —
    /// e.g. an unresolved track — is never a favorite).
    async fn is_favorite(&self, source: &ActiveSource) -> bool;

    /// Set this track's favorite state for `source` (an empty key is a no-op).
    async fn set_favorite(&self, source: &ActiveSource, on: bool) -> Result<(), SourceError>;
}

#[async_trait]
impl TrackFavorite for reader::Track {
    async fn is_favorite(&self, source: &ActiveSource) -> bool {
        let key = self.id.key();
        !key.trim().is_empty() && source.is_favorite(key.as_ref()).await
    }

    async fn set_favorite(&self, source: &ActiveSource, on: bool) -> Result<(), SourceError> {
        let key = self.id.key();
        if key.trim().is_empty() {
            return Ok(());
        }
        source.set_favorite(key.as_ref(), on).await
    }
}

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

// ============================ SoundCloud ===============================

struct SoundcloudSource {
    db: Db,
    source: Source,
    /// OAuth token for the signed-in account; `None` = anonymous (search + play
    /// of public tracks still work via the scraped web-player client_id).
    token: Option<String>,
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
