use config::MusicService;
use serde::{Deserialize, Deserializer, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Album {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub genre: String,
    pub year: u16,
    pub cover_path: Option<PathBuf>,
    #[serde(default)]
    pub manual_cover: bool,
}

/// A source-agnostic artist photo reference: a local file path or a remote URL.
/// Resolved to a `CoverUrl` by the cover seam (`server::cover::artist`), so the
/// UI never branches on where the image lives. A custom user override is handled
/// separately (it's a priority concern, not a source one).
#[derive(Debug, Clone, PartialEq)]
pub enum ArtistImageRef {
    /// A local filesystem path (from the local scan).
    Local(PathBuf),
    /// A remote URL (from a server sync).
    Remote(String),
}

/// Typed in-memory form of the cover references persisted by older databases
/// and source adapters.
///
/// Storage remains string-based for backwards compatibility. All interpretation
/// of those strings happens here so callers never need to split service prefixes
/// or decode embedded URLs themselves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoverRef {
    Local(PathBuf),
    JellyfinItem {
        item_id: String,
        tag: Option<String>,
    },
    SubsonicItem {
        item_id: String,
        /// Ask for the item's own art over a fully signed request. Set when the
        /// source recorded no cover key at all — that lookup doesn't accept the
        /// query-token form every other Subsonic ref resolves with.
        signed: bool,
    },
    EmbeddedUrl(String),
    None,
}

/// Typed track identity — replaces the old `Track.path` synthetic-string hack.
/// Local tracks are a filesystem path; server tracks are a service + item id.
/// The cover reference is a separate `Track.cover` field, NOT part of identity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum TrackId {
    Local(PathBuf),
    Server {
        service: config::MusicService,
        item_id: String,
    },
}

impl TrackId {
    /// The bare key within its source — the file-path string (local) or the
    /// item/video id (server). This is the DB `track_key`.
    pub fn key(&self) -> std::borrow::Cow<'_, str> {
        match self {
            TrackId::Local(p) => p.to_string_lossy(),
            TrackId::Server { item_id, .. } => std::borrow::Cow::Borrowed(item_id),
        }
    }

    /// The filesystem path, if this is a local track.
    pub fn local_path(&self) -> Option<&Path> {
        match self {
            TrackId::Local(p) => Some(p),
            TrackId::Server { .. } => None,
        }
    }

    /// The media service, if this is a server track.
    pub fn service(&self) -> Option<config::MusicService> {
        match self {
            TrackId::Server { service, .. } => Some(*service),
            TrackId::Local(_) => None,
        }
    }

    /// A stable, source-qualified identity string (no cover): the file path for
    /// local, or `"<service-prefix>:<item_id>"` for server. For logging /
    /// cross-source string keys.
    pub fn uid(&self) -> String {
        match self {
            TrackId::Local(p) => p.to_string_lossy().into_owned(),
            TrackId::Server { service, item_id } => {
                format!("{}:{}", service_prefix(*service), item_id)
            }
        }
    }

    /// Parse a legacy `Track.path` string (`"service:id[:cover]"` or a real
    /// path). Used ONLY by the migration importer; the 3rd cover segment is
    /// dropped here (the importer sets `Track.cover` separately).
    pub fn from_legacy_path(s: &str) -> Self {
        for (prefix, svc) in [
            ("ytmusic", config::MusicService::YtMusic),
            ("jellyfin", config::MusicService::Jellyfin),
            ("subsonic", config::MusicService::Subsonic),
            ("custom", config::MusicService::Custom),
            ("soundcloud", config::MusicService::SoundCloud),
        ] {
            if let Some(rest) = s.strip_prefix(prefix).and_then(|r| r.strip_prefix(':')) {
                let item_id = rest.split(':').next().unwrap_or("").to_string();
                return TrackId::Server {
                    service: svc,
                    item_id,
                };
            }
        }
        TrackId::Local(PathBuf::from(s))
    }
}

fn service_prefix(s: config::MusicService) -> &'static str {
    match s {
        config::MusicService::YtMusic => "ytmusic",
        config::MusicService::Jellyfin => "jellyfin",
        config::MusicService::Subsonic => "subsonic",
        config::MusicService::Custom => "custom",
        config::MusicService::SoundCloud => "soundcloud",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Track {
    pub id: TrackId,
    /// Cover art reference (URL for server, path for local) — out of identity.
    #[serde(default)]
    pub cover: Option<String>,
    pub album_id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: u64,
    pub khz: u32,
    #[serde(default)]
    pub bitrate: u16,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    #[serde(default)]
    pub musicbrainz_release_id: Option<String>,
    #[serde(default)]
    pub musicbrainz_recording_id: Option<String>,
    #[serde(default)]
    pub musicbrainz_track_id: Option<String>,
    #[serde(default)]
    pub playlist_item_id: Option<String>,
    #[serde(default)]
    pub artists: Vec<String>,
}

impl CoverRef {
    /// The persisted sentinel for "the source reported no cover for this item".
    pub const NO_COVER: &'static str = "none";

    /// Parse a persisted cover reference without consulting the active source.
    pub fn parse(stored: &str) -> Self {
        if stored.is_empty() || stored == Self::NO_COVER {
            return Self::None;
        }
        if let Some(url) = Self::decode_embedded(stored) {
            return Self::EmbeddedUrl(url);
        }

        let path = Path::new(stored);
        if path.is_absolute() {
            return Self::Local(path.to_path_buf());
        }

        let mut parts = stored.splitn(3, ':');
        let prefix = parts.next().unwrap_or_default();
        let item_id = parts.next().unwrap_or_default();
        let value = parts.next();
        match prefix {
            "jellyfin" if !item_id.is_empty() => {
                Self::remote_item(MusicService::Jellyfin, item_id, value)
            }
            "subsonic" if !item_id.is_empty() => {
                Self::remote_item(MusicService::Subsonic, item_id, value)
            }
            "custom" if !item_id.is_empty() => {
                Self::remote_item(MusicService::Custom, item_id, value)
            }
            // YT Music and legacy SoundCloud refs only carry self-contained
            // artwork. Their item identity is irrelevant to cover resolution.
            "ytmusic" | "soundcloud" => value.map_or(Self::None, Self::parse),
            _ => Self::None,
        }
    }

    /// Build a cover ref when the service and item identity are already typed.
    pub fn remote_item(service: MusicService, item_id: &str, cover: Option<&str>) -> Self {
        if let Some(value) = cover {
            if value == Self::NO_COVER {
                return Self::None;
            }
            // The value may already describe a cover on its own: an embedded
            // URL, or — when an album's whole ref is projected onto a track — a
            // ref in its own right. Either wins over reading it as *this*
            // item's key, which is the misread this type exists to prevent. A
            // real image tag carries no service prefix and no URL scheme, so it
            // parses to `None` and falls through to the arms below.
            match Self::parse(value) {
                Self::None | Self::Local(_) => {}
                typed => return typed,
            }
        }

        match service {
            MusicService::Jellyfin => Self::JellyfinItem {
                item_id: item_id.to_string(),
                tag: cover.map(str::to_string),
            },
            MusicService::Subsonic | MusicService::Custom => Self::SubsonicItem {
                item_id: item_id.to_string(),
                signed: false,
            },
            MusicService::YtMusic | MusicService::SoundCloud => {
                cover.map_or(Self::None, Self::parse)
            }
        }
    }

    /// Encode an item-scoped cover ref in the persisted `service:item[:value]`
    /// form [`parse`](Self::parse) reads back — the one place a service prefix
    /// is written. Hand-rolled `format!`s are how a Subsonic row ended up
    /// carrying a `jellyfin:` ref.
    pub fn stored_item_ref(service: MusicService, item_id: &str, cover: Option<&str>) -> String {
        let prefix = service_prefix(service);
        match cover {
            Some(value) => format!("{prefix}:{item_id}:{value}"),
            None => format!("{prefix}:{item_id}"),
        }
    }

    /// Resolve the cover candidate and fallback encoded by a typed track.
    pub fn for_track(track: &Track) -> Self {
        let Some(service) = track.id.service() else {
            return track.cover.as_deref().map_or(Self::None, Self::parse);
        };
        let item_id = track.id.key();

        match service {
            MusicService::Jellyfin => match track.cover.as_deref() {
                Some(cover) => Self::remote_item(service, &item_id, Some(cover)),
                None if track.album_id.starts_with("jellyfin:") => Self::parse(&track.album_id),
                None => Self::remote_item(service, &item_id, None),
            },
            MusicService::YtMusic => match track.cover.as_deref() {
                Some(cover) => Self::remote_item(service, &item_id, Some(cover)),
                None => Self::parse(&track.album_id),
            },
            MusicService::Subsonic | MusicService::Custom
                if track.cover.as_deref() == Some(Self::NO_COVER) =>
            {
                // The sync found no cover key for this song. Its own art is
                // still worth asking for — but only over a signed request.
                Self::SubsonicItem {
                    item_id: item_id.into_owned(),
                    signed: true,
                }
            }
            MusicService::Subsonic | MusicService::Custom => {
                Self::remote_item(service, &item_id, track.cover.as_deref())
            }
            MusicService::SoundCloud => track.cover.as_deref().map_or(Self::None, Self::parse),
        }
    }

    /// Encode a URL for persisted fields that still use the `urlhex_…` form.
    pub fn encode_url(url: &str) -> String {
        let mut encoded = String::with_capacity(7 + url.len() * 2);
        encoded.push_str("urlhex_");
        for byte in url.as_bytes() {
            use std::fmt::Write as _;
            let _ = write!(encoded, "{byte:02x}");
        }
        encoded
    }

    fn decode_embedded(stored: &str) -> Option<String> {
        if let Some(url) = stored.strip_prefix("directurl:") {
            return (!url.is_empty()).then(|| url.to_string());
        }
        if stored.starts_with("http://") || stored.starts_with("https://") {
            return Some(stored.to_string());
        }

        let hex = stored.strip_prefix("urlhex_")?;
        if hex.len() % 2 != 0 {
            return None;
        }
        let mut bytes = Vec::with_capacity(hex.len() / 2);
        for pair in hex.as_bytes().chunks_exact(2) {
            let pair = std::str::from_utf8(pair).ok()?;
            bytes.push(u8::from_str_radix(pair, 16).ok()?);
        }
        String::from_utf8(bytes).ok()
    }
}

/// What to do with the track's embedded front-cover picture on save.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum CoverChange {
    /// Leave the existing picture untouched.
    #[default]
    Keep,
    /// Strip the front-cover picture from the file.
    Remove,
    /// Replace the front cover with these image bytes (format auto-detected).
    Set(Vec<u8>),
}

/// User-supplied edits to a track's tags. Empty strings / `None` mean
/// "remove this tag from the file". Produced by the metadata editor UI and
/// consumed by [`crate::metadata::write_tags`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrackEdits {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub cover: CoverChange,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct Library {
    #[serde(
        default,
        alias = "root_path",
        deserialize_with = "deserialize_root_paths"
    )]
    pub root_paths: Vec<PathBuf>,
    pub tracks: Vec<Track>,
    pub albums: Vec<Album>,
    #[serde(default)]
    pub jellyfin_tracks: Vec<Track>,
    #[serde(default)]
    pub jellyfin_albums: Vec<Album>,
    #[serde(default)]
    pub jellyfin_genres: Vec<(String, String)>,
    /// Unix timestamp (seconds) of the last successful YT library sync.
    /// `None` means "never synced" → the Favorites page kicks off an
    /// initial fetch on next mount. Cleared by the manual refresh
    /// button to force a re-fetch.
    #[serde(default)]
    pub last_yt_sync_at: Option<u64>,
    /// Companion to `last_yt_sync_at` for the YT playlists list.
    /// Tracked separately because the favorites page and the playlists
    /// page are independent — one synced doesn't imply the other.
    #[serde(default)]
    pub last_yt_playlists_sync_at: Option<u64>,
    #[serde(default)]
    pub server_artist_images: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub local_artist_images: std::collections::HashMap<String, PathBuf>,
    /// User-set custom artist photos, keyed by normalized (trim+lowercase) artist name.
    /// Overrides both local_artist_images and server_artist_images when present.
    #[serde(default)]
    pub custom_artist_images: std::collections::HashMap<String, PathBuf>,
}

fn deserialize_root_paths<'de, D>(deserializer: D) -> Result<Vec<PathBuf>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(PathBuf),
        Many(Vec<PathBuf>),
    }
    match OneOrMany::deserialize(deserializer)? {
        OneOrMany::One(p) => Ok(vec![p]),
        OneOrMany::Many(v) => Ok(v),
    }
}

impl Library {
    pub fn new(root_paths: Vec<PathBuf>) -> Self {
        Self {
            root_paths,
            ..Default::default()
        }
    }

    pub fn add_track(&mut self, track: Track) {
        if let Some(index) = self.tracks.iter().position(|t| t.id == track.id) {
            self.tracks[index] = track;
        } else {
            self.tracks.push(track);
        }
    }

    pub fn add_album(&mut self, album: Album) {
        if let Some(index) = self.albums.iter().position(|a| a.id == album.id) {
            let mut new_album = album;
            let existing = &self.albums[index];
            if new_album.cover_path.is_none() || existing.manual_cover {
                new_album.cover_path = existing.cover_path.clone();
            }
            if existing.manual_cover {
                new_album.manual_cover = true;
            }
            self.albums[index] = new_album;
        } else {
            self.albums.push(album);
        }
    }

    pub fn remove_track(&mut self, id: &TrackId) {
        self.tracks.retain(|t| &t.id != id);
    }

    pub fn remove_album(&mut self, album_id: &str) {
        self.albums.retain(|a| a.id != album_id);
        self.tracks.retain(|t| t.album_id != album_id);
    }
}

#[cfg(test)]
mod tests {
    use super::{CoverRef, Library, Track, TrackId};
    use config::MusicService;
    use std::path::PathBuf;

    #[test]
    fn library_deserializes_legacy_root_path() {
        let json = r#"{
            "root_path": "/music",
            "tracks": [],
            "albums": []
        }"#;

        let library: Library = serde_json::from_str(json).unwrap();

        assert_eq!(library.root_paths, vec![PathBuf::from("/music")]);
    }

    #[test]
    fn cover_ref_parses_every_persisted_shape() {
        assert_eq!(
            CoverRef::parse("/music/album/cover.jpg"),
            CoverRef::Local(PathBuf::from("/music/album/cover.jpg"))
        );
        assert_eq!(
            CoverRef::parse("jellyfin:album-1:tag-1"),
            CoverRef::JellyfinItem {
                item_id: "album-1".to_string(),
                tag: Some("tag-1".to_string())
            }
        );
        assert_eq!(
            CoverRef::parse("subsonic:cover-1"),
            CoverRef::SubsonicItem {
                item_id: "cover-1".to_string(),
                signed: false
            }
        );
        assert_eq!(
            CoverRef::parse("directurl:https://img.example/cover.jpg"),
            CoverRef::EmbeddedUrl("https://img.example/cover.jpg".to_string())
        );

        let url = "https://img.example/a:b.jpg";
        let encoded = CoverRef::encode_url(url);
        assert_eq!(
            CoverRef::parse(&format!("ytmusic:_:{encoded}")),
            CoverRef::EmbeddedUrl(url.to_string())
        );
        assert_eq!(CoverRef::parse("jellyfin:album-1:none"), CoverRef::None);
        assert_eq!(CoverRef::parse("urlhex_not-hex"), CoverRef::None);
    }

    fn track(service: MusicService, item_id: &str, cover: Option<&str>, album_id: &str) -> Track {
        Track {
            id: TrackId::Server {
                service,
                item_id: item_id.to_string(),
            },
            cover: cover.map(str::to_string),
            album_id: album_id.to_string(),
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            duration: 0,
            khz: 0,
            bitrate: 0,
            track_number: None,
            disc_number: None,
            musicbrainz_release_id: None,
            musicbrainz_recording_id: None,
            musicbrainz_track_id: None,
            playlist_item_id: None,
            artists: Vec::new(),
        }
    }

    #[test]
    fn jellyfin_track_uses_typed_album_fallback() {
        let with_tag = track(
            MusicService::Jellyfin,
            "track-1",
            None,
            "jellyfin:album-1:album-tag",
        );
        assert_eq!(
            CoverRef::for_track(&with_tag),
            CoverRef::JellyfinItem {
                item_id: "album-1".to_string(),
                tag: Some("album-tag".to_string())
            }
        );

        // A bare album ref (the sync's form when the album has no Primary tag)
        // still names the album, not the song with no art of its own.
        let bare = track(MusicService::Jellyfin, "track-1", None, "jellyfin:album-1");
        assert_eq!(
            CoverRef::for_track(&bare),
            CoverRef::JellyfinItem {
                item_id: "album-1".to_string(),
                tag: None
            }
        );

        // No album ref at all → the song's own item.
        let orphan = track(MusicService::Jellyfin, "track-1", None, "");
        assert_eq!(
            CoverRef::for_track(&orphan),
            CoverRef::JellyfinItem {
                item_id: "track-1".to_string(),
                tag: None
            }
        );
    }

    /// The regression the type exists to prevent: an album's whole ref landing
    /// in a track's `cover` slot must not be sent as that *track's* image tag.
    #[test]
    fn a_ref_in_the_cover_slot_resolves_as_that_ref() {
        assert_eq!(
            CoverRef::remote_item(
                MusicService::Jellyfin,
                "track-1",
                Some("jellyfin:album-1:album-tag")
            ),
            CoverRef::JellyfinItem {
                item_id: "album-1".to_string(),
                tag: Some("album-tag".to_string())
            }
        );

        // A real Jellyfin image tag carries no service prefix, so it stays the
        // track's own tag.
        assert_eq!(
            CoverRef::remote_item(MusicService::Jellyfin, "track-1", Some("d41d8cd98f00b204")),
            CoverRef::JellyfinItem {
                item_id: "track-1".to_string(),
                tag: Some("d41d8cd98f00b204".to_string())
            }
        );
    }

    #[test]
    fn subsonic_track_without_a_cover_key_asks_for_its_own_art() {
        assert_eq!(
            CoverRef::for_track(&track(
                MusicService::Subsonic,
                "song-1",
                Some(CoverRef::NO_COVER),
                ""
            )),
            CoverRef::SubsonicItem {
                item_id: "song-1".to_string(),
                signed: true
            }
        );

        // Absent (rather than sentinel) → the plain token-authenticated lookup.
        assert_eq!(
            CoverRef::for_track(&track(MusicService::Custom, "song-1", None, "")),
            CoverRef::SubsonicItem {
                item_id: "song-1".to_string(),
                signed: false
            }
        );
    }

    #[test]
    fn soundcloud_track_covers_are_self_contained() {
        let url = "https://i1.sndcdn.com/artworks-1:2-large.jpg";
        for stored in [url.to_string(), format!("directurl:{url}")] {
            assert_eq!(
                CoverRef::for_track(&track(MusicService::SoundCloud, "sc-1", Some(&stored), "")),
                CoverRef::EmbeddedUrl(url.to_string())
            );
        }
    }

    #[test]
    fn stored_item_refs_round_trip_through_parse() {
        for (service, expected) in [
            (
                MusicService::Jellyfin,
                CoverRef::JellyfinItem {
                    item_id: "item-1".to_string(),
                    tag: Some("tag-1".to_string()),
                },
            ),
            (
                MusicService::Subsonic,
                CoverRef::SubsonicItem {
                    item_id: "item-1".to_string(),
                    signed: false,
                },
            ),
            (
                MusicService::Custom,
                CoverRef::SubsonicItem {
                    item_id: "item-1".to_string(),
                    signed: false,
                },
            ),
        ] {
            let stored = CoverRef::stored_item_ref(service, "item-1", Some("tag-1"));
            assert_eq!(CoverRef::parse(&stored), expected, "{stored}");
            assert_eq!(
                CoverRef::parse(&CoverRef::stored_item_ref(
                    service,
                    "item-1",
                    Some(CoverRef::NO_COVER)
                )),
                CoverRef::None,
                "the no-cover sentinel survives the round trip"
            );
        }

        let url = "https://img.example/a:b.jpg";
        let stored =
            CoverRef::stored_item_ref(MusicService::YtMusic, "_", Some(&CoverRef::encode_url(url)));
        assert_eq!(CoverRef::parse(&stored), CoverRef::EmbeddedUrl(url.into()));
    }
}

/// One playlist. `tracks` are opaque refs — a filesystem path string for local
/// playlists, an item/video id for a server. Which source these belong to is
/// context (the active source the store was loaded for), not per-row state, so
/// there's no source field and no local/server type split. The path↔file
/// conversion happens only at the player's resolve boundary, not here.
#[derive(Debug, Clone, PartialEq)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub tracks: Vec<String>,
    /// Server cover-version tag (server playlists only; `None` for local).
    pub image_tag: Option<String>,
    pub cover_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlaylistFolder {
    pub id: String,
    pub name: String,
    pub playlist_ids: Vec<String>,
}

/// The in-memory playlist read model for the active source (built by the DB
/// layer, never serialized). One uniform list — local vs server is the active
/// source context, not a per-row split.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlaylistStore {
    pub playlists: Vec<Playlist>,
    pub folders: Vec<PlaylistFolder>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct FavoritesStore {
    #[serde(default)]
    pub local_favorites: Vec<PathBuf>,
    #[serde(default)]
    pub jellyfin_favorites: Vec<String>,
}

impl FavoritesStore {
    pub fn is_local_favorite(&self, path: &Path) -> bool {
        self.local_favorites.iter().any(|p| p == path)
    }

    pub fn is_jellyfin_favorite(&self, id: &str) -> bool {
        self.jellyfin_favorites.iter().any(|i| i == id)
    }

    pub fn toggle_local(&mut self, path: PathBuf) -> bool {
        if let Some(pos) = self.local_favorites.iter().position(|p| p == &path) {
            self.local_favorites.remove(pos);
            false
        } else {
            self.local_favorites.push(path);
            true
        }
    }

    pub fn set_jellyfin(&mut self, id: String, is_fav: bool) {
        if is_fav {
            if !self.jellyfin_favorites.contains(&id) {
                self.jellyfin_favorites.push(id);
            }
        } else {
            self.jellyfin_favorites.retain(|i| i != &id);
        }
    }
}
