//! The plugin wire protocol: JSON-RPC 2.0 over newline-delimited JSON on the
//! child's stdio.
//!
//! This module is the schema and nothing else — no I/O, no policy. Every type
//! here is one side of a message on the pipe, so a change to any of them is a
//! change to [`PROTOCOL_VERSION`]. `docs/plugins.md` is the prose form of this
//! file; keep the two in step.
//!
//! Framing: one compact JSON object per line. `stdin` carries host→plugin,
//! `stdout` carries plugin→host, and `stderr` is free-form plugin logging the
//! host re-emits through `tracing`. Requests carry an integer `id` and are
//! answered by exactly one [`Response`] with the same id; [`Notification`]s
//! carry no id and are never answered.
//!
//! The DTOs are deliberately *not* the app's own model types. A plugin speaks
//! ids and strings; the host does the id namespacing and the mapping into
//! `reader::Track` and friends in `source::plugin`. That keeps a
//! refactor of the internal models from breaking every installed plugin.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::source::Capabilities;

/// The protocol revision this build speaks. A plugin reporting anything else in
/// its handshake is rejected rather than probed for compatibility — silently
/// half-speaking an older dialect is how plugin systems rot.
pub const PROTOCOL_VERSION: u32 = 1;

/// Longest single line the host will read from a plugin. A plugin that exceeds
/// it is killed: at that size the stream is either corrupt or hostile, and the
/// alternative is an unbounded allocation driven by the child.
pub const MAX_LINE_BYTES: usize = 16 * 1024 * 1024;

// =============================== Envelope ================================

/// A host→plugin call. `id` is monotonically increasing per connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

impl Request {
    pub fn new(id: u64, method: impl Into<String>, params: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.into(),
            params,
        }
    }
}

/// A plugin→host reply. Exactly one of `result`/`error` is set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    #[serde(default)]
    pub jsonrpc: String,
    pub id: u64,
    #[serde(default)]
    pub result: Option<serde_json::Value>,
    #[serde(default)]
    pub error: Option<RpcError>,
}

/// An unanswered message. Flows both ways: host→plugin `shutdown` /
/// `auth_cancel`, plugin→host `log` / `auth_changed` / `library_changed`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    #[serde(default)]
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

impl Notification {
    pub fn new(method: impl Into<String>, params: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method: method.into(),
            params,
        }
    }
}

/// Either shape a plugin's stdout line can take. Untagged so one `serde_json`
/// pass classifies the line; a [`Response`] is tried first because it is the
/// only one carrying `id`.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Incoming {
    Response(Response),
    Notification(Notification),
}

/// The JSON-RPC error object. `data.kind` is what the host branches on;
/// `code`/`message` are for humans and logs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(default)]
    pub data: Option<ErrorData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorData {
    pub kind: ErrorKind,
}

/// The five failure classes, mapping 1:1 onto [`crate::source::SourceError`] so
/// the UI reacts the same way it does for a built-in source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// The plugin does not implement this operation.
    Unsupported,
    /// The plugin could not reach its backend.
    Connectivity,
    /// The plugin is not signed in, or its credentials expired.
    Auth,
    /// The host asked for something malformed.
    InvalidInput,
    /// Anything else the plugin failed at.
    Backend,
}

// ============================== Lifecycle ================================

/// Method name of every host→plugin message. String constants rather than an
/// enum so an unknown method from a newer host is a plugin-side error, not an
/// undecodable line.
pub mod method {
    pub const INITIALIZE: &str = "initialize";
    pub const PING: &str = "ping";
    pub const SHUTDOWN: &str = "shutdown";
    pub const AUTH_BEGIN: &str = "auth_begin";
    pub const AUTH_SUBMIT: &str = "auth_submit";
    pub const AUTH_CANCEL: &str = "auth_cancel";
    pub const VALIDATE: &str = "validate";
    pub const RESOLVE_STREAM: &str = "resolve_stream";
    pub const FETCH_FAVORITES: &str = "fetch_favorites";
    pub const FETCH_FAVORITES_PAGE: &str = "fetch_favorites_page";
    pub const PUSH_FAVORITE: &str = "push_favorite";
    pub const FETCH_PLAYLISTS: &str = "fetch_playlists";
    pub const FETCH_PLAYLIST_ENTRIES_PAGE: &str = "fetch_playlist_entries_page";
    pub const ADD_TO_PLAYLIST: &str = "add_to_playlist";
    pub const CREATE_PLAYLIST: &str = "create_playlist";
    pub const REMOVE_FROM_PLAYLIST: &str = "remove_from_playlist";
    pub const REORDER_PLAYLIST: &str = "reorder_playlist";
    pub const SEARCH: &str = "search";
    pub const FETCH_ALBUM: &str = "fetch_album";
    pub const FETCH_ALBUM_TRACKS: &str = "fetch_album_tracks";
    pub const FETCH_ALBUM_BY_REF: &str = "fetch_album_by_ref";
    pub const FETCH_ALBUM_BY_META: &str = "fetch_album_by_meta";
    pub const FETCH_ARTIST: &str = "fetch_artist";
    pub const RESOLVE_ARTIST_ID: &str = "resolve_artist_id";
    pub const RESOLVE_ALBUM_ID: &str = "resolve_album_id";
    pub const START_RADIO: &str = "start_radio";
    pub const DISCOVER_HOME: &str = "discover_home";
    pub const DISCOVER_CONTINUATION: &str = "discover_continuation";
    pub const FETCH_LIBRARY: &str = "fetch_library";
    pub const FETCH_ARTIST_IMAGES: &str = "fetch_artist_images";
    pub const FETCH_ARTIST_IMAGE: &str = "fetch_artist_image";
}

/// Method name of every plugin→host notification.
pub mod event {
    pub const LOG: &str = "log";
    pub const AUTH_CHANGED: &str = "auth_changed";
    pub const LIBRARY_CHANGED: &str = "library_changed";
}

/// `initialize` params — everything the plugin needs before it can answer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeParams {
    pub protocol: u32,
    pub host_version: String,
    /// BCP-47-ish language tag of the running UI, for plugin-authored strings.
    pub locale: String,
    /// The plugin's private state directory. The host creates it and never
    /// reads or deletes it — including when the source is removed.
    pub data_dir: PathBuf,
}

/// `initialize` result — the handshake. Capabilities live here rather than in
/// the manifest so they cannot drift from what the running binary can do.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeResult {
    pub protocol: u32,
    pub name: String,
    pub version: String,
    pub capabilities: Capabilities,
    /// True when the plugin needs the auth wizard run before it can serve.
    #[serde(default)]
    pub auth_required: bool,
    /// Origin of the plugin's own byte server, e.g. `http://127.0.0.1:51234`.
    /// The host never parses this — it only ever plays back what
    /// [`StreamResult::url`] contains.
    #[serde(default)]
    pub data_base_url: String,
    /// Per-process secret the plugin embeds in the stream URLs it returns. Held
    /// only so the host can redact it from logs.
    #[serde(default)]
    pub data_token: String,
    /// A human-readable account label for the settings row, when signed in.
    #[serde(default)]
    pub account: Option<String>,
    /// Optional `https://…/{id}` template for "open on the web". `{id}` is
    /// replaced with the bare item id. Absent means the source has no web page.
    #[serde(default)]
    pub web_url_template: Option<String>,
}

/// `log` notification params. The host re-emits these through `tracing` under
/// target `plugin`, with the plugin id as a field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogParams {
    #[serde(default)]
    pub level: LogLevel,
    pub message: String,
    #[serde(default)]
    pub target: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Trace,
    Debug,
    #[default]
    Info,
    Warn,
    Error,
}

/// `auth_changed` notification params.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthChangedParams {
    pub authenticated: bool,
}

// ================================ Auth ===================================

/// One step of the plugin-authored sign-in wizard. The host renders whichever
/// variant arrives with a single generic popup and posts the collected values
/// back through `auth_submit`, looping until [`Done`](AuthPrompt::Done) or
/// [`Failed`](AuthPrompt::Failed). Kopuz never learns what the plugin is asking
/// for — that is the whole point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthPrompt {
    /// Send the user to a URL (OAuth). The next `auth_submit` carries no values
    /// and is the host saying "the user clicked continue".
    OpenUrl { url: String, message: String },
    /// Collect the declared fields. `auth_submit` carries `key → value`.
    Form {
        title: String,
        fields: Vec<AuthField>,
    },
    /// Show text and wait for acknowledgement.
    Message { text: String },
    /// Sign-in complete.
    Done,
    /// Sign-in failed; `message` is shown verbatim.
    Failed { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthField {
    pub key: String,
    pub label: String,
    /// Render as a password input and keep out of logs.
    #[serde(default)]
    pub secret: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuthSubmitParams {
    #[serde(default)]
    pub values: HashMap<String, String>,
}

// ================================ Data ===================================

/// A track as a plugin describes it. `item_id` is the plugin's own id — the
/// host prefixes it with the plugin id before it ever reaches the database.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginTrack {
    pub item_id: String,
    pub title: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub artists: Vec<String>,
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub album_id: String,
    /// A cover reference. Use `directurl:https://…` (or the `urlhex_<hex>`
    /// form) — those resolve with no server configured, which is what makes
    /// plugin covers need zero host-side code.
    #[serde(default)]
    pub cover: Option<String>,
    #[serde(default)]
    pub duration_secs: u64,
    #[serde(default)]
    pub khz: u32,
    #[serde(default)]
    pub bitrate: u16,
    #[serde(default)]
    pub track_number: Option<u32>,
    #[serde(default)]
    pub disc_number: Option<u32>,
    /// The playlist-entry handle, when this track came from a playlist and the
    /// backend needs it to remove the entry.
    #[serde(default)]
    pub playlist_item_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginAlbum {
    pub album_id: String,
    pub title: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub year: Option<u16>,
    #[serde(default)]
    pub cover: Option<String>,
    #[serde(default)]
    pub genre: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginAlbumDetail {
    pub album: PluginAlbum,
    #[serde(default)]
    pub tracks: Vec<PluginTrack>,
    /// An opaque handle the plugin accepts back as "play this whole album".
    #[serde(default)]
    pub play_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginArtist {
    pub artist_id: String,
    pub name: String,
    #[serde(default)]
    pub image: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginArtistPage {
    pub artist_id: String,
    pub name: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub banner: Option<String>,
    #[serde(default)]
    pub shuffle_ref: Option<String>,
    #[serde(default)]
    pub shelves: Vec<PluginShelf>,
}

/// One horizontal carousel (or vertical song list) on a discover/artist page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginShelf {
    pub title: String,
    #[serde(default)]
    pub strapline: Option<String>,
    /// Opaque token the host passes back to `discover_continuation` for a
    /// "see all" of this shelf.
    #[serde(default)]
    pub more_ref: Option<String>,
    /// Render as a numbered song list instead of a tile carousel.
    #[serde(default)]
    pub is_song_list: bool,
    #[serde(default)]
    pub items: Vec<PluginShelfItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PluginShelfItem {
    Song(Box<PluginTrack>),
    Album {
        album_id: String,
        title: String,
        #[serde(default)]
        subtitle: String,
        #[serde(default)]
        cover: Option<String>,
    },
    Artist {
        artist_id: String,
        name: String,
        #[serde(default)]
        image: Option<String>,
    },
    Playlist {
        playlist_id: String,
        title: String,
        #[serde(default)]
        subtitle: String,
        #[serde(default)]
        cover: Option<String>,
    },
    Category {
        id: String,
        title: String,
        #[serde(default)]
        cover: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginPlaylistMeta {
    pub playlist_id: String,
    pub name: String,
    #[serde(default)]
    pub image: Option<String>,
}

/// One page of anything. `next` is an opaque cursor the host hands straight
/// back; `None` ends the walk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Page<T> {
    #[serde(default = "Vec::new")]
    pub items: Vec<T>,
    #[serde(default)]
    pub next: Option<String>,
}

impl<T> Default for Page<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            next: None,
        }
    }
}

/// `fetch_library` result — a whole-library snapshot for the sync task.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LibraryResult {
    #[serde(default)]
    pub albums: Vec<PluginAlbum>,
    #[serde(default)]
    pub tracks: Vec<PluginTrack>,
    /// `(artist name, image URL)` pairs.
    #[serde(default)]
    pub artist_images: Vec<(String, String)>,
}

/// `search` result.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchResult {
    #[serde(default)]
    pub tracks: Vec<PluginTrack>,
    #[serde(default)]
    pub albums: Vec<PluginAlbum>,
}

/// `discover_home` / `discover_continuation` result.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiscoverResult {
    #[serde(default)]
    pub shelves: Vec<PluginShelf>,
    #[serde(default)]
    pub next: Option<String>,
}

/// `resolve_stream` result. Omitting `format` selects the host's default
/// buffered-GET path, which is what a plugin serving its own bytes wants: a
/// plain `GET` answered with 2xx, real audio, and an accurate `Content-Length`
/// (required for scrubbing). The URL must not look like a `.pls`/`.m3u`
/// playlist or the host will try to follow it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamResult {
    pub url: String,
    #[serde(default)]
    pub content_length: Option<u64>,
    #[serde(default)]
    pub duration_secs: Option<u64>,
    #[serde(default)]
    pub bitrate: Option<u32>,
}

// ============================ Request params =============================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemIdParams {
    pub item_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CursorParams {
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushFavoriteParams {
    pub item_id: String,
    pub on: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistPageParams {
    pub playlist_id: String,
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddToPlaylistParams {
    pub playlist_id: String,
    pub item_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePlaylistParams {
    pub name: String,
    pub item_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoveFromPlaylistParams {
    pub playlist_id: String,
    pub item_id: String,
    /// The entry handle from [`PluginTrack::playlist_item_id`], when the
    /// backend removes by entry rather than by track.
    #[serde(default)]
    pub playlist_item_id: Option<String>,
    pub position: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReorderPlaylistParams {
    pub playlist_id: String,
    /// The full new membership, in order. The host does not track the old
    /// order, so this — not a `from` index — is what says where things moved.
    pub ordered_ids: Vec<String>,
    /// The one entry that changed position.
    pub item_id: String,
    #[serde(default)]
    pub playlist_item_id: Option<String>,
    /// Its new index within `ordered_ids`.
    pub to: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchParams {
    pub query: String,
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlbumIdParams {
    pub album_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlbumRefParams {
    pub album_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlbumMetaParams {
    pub title: String,
    pub artist: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtistIdParams {
    pub artist_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryParams {
    pub query: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NameParams {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedParams {
    pub seed_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenParams {
    pub token: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_kind_wire_names() {
        let json = serde_json::to_string(&ErrorKind::InvalidInput).expect("serialize");
        assert_eq!(json, "\"invalid_input\"");
    }

    #[test]
    fn auth_prompt_round_trips() {
        let prompt = AuthPrompt::Form {
            title: "Sign in".into(),
            fields: vec![AuthField {
                key: "password".into(),
                label: "Password".into(),
                secret: true,
            }],
        };
        let json = serde_json::to_string(&prompt).expect("serialize");
        let back: AuthPrompt = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(prompt, back);
        assert_eq!(
            serde_json::to_string(&AuthPrompt::Done).expect("serialize"),
            "\"Done\""
        );
    }

    #[test]
    fn incoming_discriminates_on_id() {
        let resp = r#"{"jsonrpc":"2.0","id":7,"result":null}"#;
        assert!(matches!(
            serde_json::from_str::<Incoming>(resp).expect("decode"),
            Incoming::Response(_)
        ));
        let notif = r#"{"jsonrpc":"2.0","method":"library_changed","params":{}}"#;
        assert!(matches!(
            serde_json::from_str::<Incoming>(notif).expect("decode"),
            Incoming::Notification(_)
        ));
    }

    #[test]
    fn capabilities_decode_from_handshake_json() {
        let json = r#"{
            "edit_tags": false, "delete_from_disk": false, "scan_folders": false,
            "folders": false, "sync": true, "downloads": false, "discover": true,
            "radio": true, "playlists": "AddRemove", "artist_view": "Remote",
            "albums": "Standard", "favorites_sync": "Paginated"
        }"#;
        let caps: Capabilities = serde_json::from_str(json).expect("decode");
        assert!(caps.sync);
        assert_eq!(caps.playlists, crate::source::PlaylistOps::AddRemove);
        assert_eq!(caps.favorites_sync, crate::source::FavoritesSync::Paginated);
    }

    #[test]
    fn capabilities_tolerate_missing_fields() {
        let caps: Capabilities = serde_json::from_str("{}").expect("decode");
        assert!(!caps.sync);
        assert_eq!(caps.playlists, crate::source::PlaylistOps::None);
    }

    #[test]
    fn plugin_track_needs_only_id_and_title() {
        let track: PluginTrack =
            serde_json::from_str(r#"{"item_id":"a","title":"b"}"#).expect("decode");
        assert_eq!(track.item_id, "a");
        assert!(track.artists.is_empty());
    }
}
