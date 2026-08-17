//! The tables a plugin hands back, and the ones the host hands in.
//!
//! Every type here is deserialized straight out of a Lua value by mlua's serde
//! bridge, so the field names below *are* the Lua keys. `#[serde(default)]` is
//! load-bearing: a `nil` field is an omitted field, which is how a plugin
//! written against an older API generation keeps working.
//!
//! Tagged unions use `kind`, e.g. `{ kind = "open_url", url = "…" }`. Kopuz
//! never translates these back to Lua: the flow is always plugin to host.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::source::Capabilities;

/// What `setup(ctx)` returns: the handshake. Capabilities live here rather than
/// in the manifest so they cannot drift from what the loaded script can do.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Handshake {
    /// Overrides the manifest name in the UI when present.
    pub name: Option<String>,
    pub version: Option<String>,
    pub capabilities: Capabilities,
    /// True when the sign-in wizard must run before this source can serve.
    pub auth_required: bool,
    /// A human-readable account label for the settings row, when signed in.
    pub account: Option<String>,
    /// Optional `https://…/{id}` template for "open on the web". `{id}` is
    /// replaced with the bare item id. Absent means the source has no web page.
    pub web_url_template: Option<String>,
}

/// One step of the plugin-authored sign-in wizard. The host renders whichever
/// variant arrives with a single generic popup and posts the collected values
/// back through `auth_submit`, looping until [`Done`](AuthPrompt::Done) or
/// [`Failed`](AuthPrompt::Failed). Kopuz never learns what the plugin is asking
/// for, which is the whole point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthPrompt {
    /// Send the user to a URL (OAuth). The next `auth_submit` carries no values
    /// and is the host saying "the user clicked continue".
    OpenUrl { url: String, message: String },
    /// Collect the declared fields. `auth_submit` receives `key → value`.
    Form {
        title: String,
        #[serde(default)]
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

/// The `key → value` map an `auth_submit` carries. Reaches Lua as a table.
pub type AuthValues = HashMap<String, String>;

/// A track as a plugin describes it. `item_id` is the plugin's own id, and the
/// host prefixes it with the plugin id before it ever reaches the database.
#[derive(Debug, Clone, PartialEq, Deserialize)]
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
    /// form). Those resolve with no server configured, which is what makes
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

#[derive(Debug, Clone, PartialEq, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PluginAlbumDetail {
    pub album: PluginAlbum,
    #[serde(default)]
    pub tracks: Vec<PluginTrack>,
    /// An opaque handle the plugin accepts back as "play this whole album".
    #[serde(default)]
    pub play_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PluginShelfItem {
    Song {
        track: Box<PluginTrack>,
    },
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

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PluginPlaylistMeta {
    pub playlist_id: String,
    pub name: String,
    #[serde(default)]
    pub image: Option<String>,
}

/// One page of tracks. `next` is an opaque cursor the host hands straight back;
/// `nil` ends the walk.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TrackPage {
    pub tracks: Vec<PluginTrack>,
    pub next: Option<String>,
}

/// `fetch_library` result: a whole-library snapshot for the sync task.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct LibraryResult {
    pub albums: Vec<PluginAlbum>,
    pub tracks: Vec<PluginTrack>,
    /// `{ name = "…", image = "https://…" }` entries.
    pub artist_images: Vec<ArtistImage>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ArtistImage {
    pub name: String,
    pub image: String,
}

/// `search` result.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SearchResult {
    pub tracks: Vec<PluginTrack>,
    pub albums: Vec<PluginAlbum>,
}

/// `discover_home` / `discover_continuation` result.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct DiscoverResult {
    pub shelves: Vec<PluginShelf>,
    pub next: Option<String>,
}

/// `resolve_stream` result. The host plays `url` back with its default buffered
/// GET path, so the URL must answer a plain `GET` with 2xx, real audio bytes and
/// an accurate `Content-Length` (scrubbing needs it). It must not look like a
/// `.pls`/`.m3u` playlist or the host will try to follow it instead.
#[derive(Debug, Clone, Deserialize)]
pub struct StreamResult {
    pub url: String,
    #[serde(default)]
    pub content_length: Option<u64>,
    #[serde(default)]
    pub duration_secs: Option<u64>,
    #[serde(default)]
    pub bitrate: Option<u32>,
    /// Sent as the `User-Agent` when fetching the stream, for backends that
    /// reject the default one.
    #[serde(default)]
    pub user_agent: Option<String>,
}

/// What `validate()` answers. Anything unrecognised is treated as
/// [`Unreachable`](crate::source::AuthOutcome::Unreachable), because a plugin that
/// returns nonsense must not be reported as signed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthState {
    Valid,
    Expired,
    Unreachable,
}

impl From<AuthState> for crate::source::AuthOutcome {
    fn from(s: AuthState) -> Self {
        match s {
            AuthState::Valid => Self::Valid,
            AuthState::Expired => Self::Expired,
            AuthState::Unreachable => Self::Unreachable,
        }
    }
}

/// Exported function names the host looks for on the table a plugin returns.
/// Referenced by name everywhere so a typo is a compile error, not a plugin that
/// silently reports every operation as unsupported.
pub mod export {
    pub const SETUP: &str = "setup";
    pub const UNLOAD: &str = "unload";

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
    pub const FETCH_LIBRARY: &str = "fetch_library";

    pub const FETCH_ALBUM: &str = "fetch_album";
    pub const FETCH_ALBUM_TRACKS: &str = "fetch_album_tracks";
    pub const FETCH_ALBUM_BY_REF: &str = "fetch_album_by_ref";
    pub const FETCH_ALBUM_BY_META: &str = "fetch_album_by_meta";
    pub const RESOLVE_ALBUM_ID: &str = "resolve_album_id";

    pub const FETCH_ARTIST: &str = "fetch_artist";
    pub const FETCH_ARTIST_IMAGES: &str = "fetch_artist_images";
    pub const FETCH_ARTIST_IMAGE: &str = "fetch_artist_image";
    pub const RESOLVE_ARTIST_ID: &str = "resolve_artist_id";

    pub const START_RADIO: &str = "start_radio";
    pub const DISCOVER_HOME: &str = "discover_home";
    pub const DISCOVER_CONTINUATION: &str = "discover_continuation";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_prompt_is_tagged_on_kind() {
        let json = serde_json::to_string(&AuthPrompt::Done).expect("serialize");
        assert_eq!(json, r#"{"kind":"done"}"#);
        let back: AuthPrompt =
            serde_json::from_str(r#"{"kind":"open_url","url":"u","message":"m"}"#)
                .expect("deserialize");
        assert_eq!(
            back,
            AuthPrompt::OpenUrl {
                url: "u".into(),
                message: "m".into()
            }
        );
    }

    #[test]
    fn plugin_track_needs_only_id_and_title() {
        let track: PluginTrack =
            serde_json::from_str(r#"{"item_id":"a","title":"b"}"#).expect("decode");
        assert_eq!(track.item_id, "a");
        assert!(track.artists.is_empty());
    }

    #[test]
    fn capabilities_tolerate_missing_fields() {
        let caps: Capabilities = serde_json::from_str("{}").expect("decode");
        assert!(!caps.sync);
        assert_eq!(caps.playlists, crate::source::PlaylistOps::None);
    }
}
