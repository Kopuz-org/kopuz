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

    pub fn is_server(&self) -> bool {
        matches!(self, TrackId::Server { .. })
    }

    pub fn is_local(&self) -> bool {
        matches!(self, TrackId::Local(_))
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

    /// The [`uid`](Self::uid) wrapped in a `PathBuf` — for the still-`PathBuf`-keyed
    /// selection/menu collections during the migration. (Transitional.)
    pub fn uid_path(&self) -> PathBuf {
        PathBuf::from(self.uid())
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
    use super::Library;
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub tracks: Vec<PathBuf>,
    #[serde(default)]
    pub cover_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JellyfinPlaylist {
    pub id: String,
    pub name: String,
    pub tracks: Vec<String>,
    #[serde(default)]
    pub image_tag: Option<String>,
    #[serde(default)]
    pub cover_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlaylistFolder {
    pub id: String,
    pub name: String,
    pub playlist_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct PlaylistStore {
    pub playlists: Vec<Playlist>,
    #[serde(default)]
    pub jellyfin_playlists: Vec<JellyfinPlaylist>,
    #[serde(default)]
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
