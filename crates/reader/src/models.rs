use serde::{Deserialize, Deserializer, Serialize};
use std::fs;
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
    /// Unix timestamp (seconds) when this album was added to the library.
    /// Local: newest member track's file mtime. Jellyfin: `DateCreated`.
    /// Subsonic: `created`. `None` when the source provides no date
    /// (e.g. YT Music) — sorts last under "Date Added".
    #[serde(default)]
    pub date_added: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Track {
    pub path: PathBuf,
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
    /// Unix timestamp (seconds) when this track was added to the library.
    /// Local: file mtime. Jellyfin: `DateCreated`. Subsonic: `created`.
    /// `None` when the source provides no date (e.g. YT Music) — sorts
    /// last under "Date Added".
    #[serde(default)]
    pub date_added: Option<i64>,
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

    #[tracing::instrument(name = "library.load", skip_all)]
    pub fn load(path: &Path) -> std::io::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let data = fs::read_to_string(path)?;
        let mut library: Self = serde_json::from_str(&data)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        library.backfill_local_dates();
        tracing::debug!(
            bytes = data.len(),
            tracks = library.tracks.len(),
            "library loaded from disk"
        );
        Ok(library)
    }

    /// Populate `date_added` for local tracks/albums that lack it, from file
    /// modification times. Caches written before `date_added` existed
    /// deserialize every entry to `None`, which would make the "Date Added"
    /// sort inert; the incremental scanner skips files already in the library,
    /// so a normal rescan never backfills them. This migrates them on load
    /// instead. Self-disabling once the values are populated and saved, and a
    /// no-op for remote sources (their stored paths are not real files, so
    /// `file_mtime_secs` yields `None`).
    fn backfill_local_dates(&mut self) {
        for track in self.tracks.iter_mut() {
            if track.date_added.is_none() {
                track.date_added = crate::metadata::file_mtime_secs(&track.path);
            }
        }
        // Derive each undated album's date from one of its member tracks.
        let mut album_dates: std::collections::HashMap<&str, i64> =
            std::collections::HashMap::new();
        for track in self.tracks.iter() {
            if let Some(date) = track.date_added {
                album_dates.entry(track.album_id.as_str()).or_insert(date);
            }
        }
        for album in self.albums.iter_mut() {
            if album.date_added.is_none() {
                album.date_added = album_dates.get(album.id.as_str()).copied();
            }
        }
    }

    #[tracing::instrument(name = "library.save", skip_all, fields(tracks = self.tracks.len()))]
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_string(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        tracing::debug!(bytes = data.len(), "writing library to disk");
        fs::write(path, data)
    }

    pub fn add_track(&mut self, track: Track) {
        if let Some(index) = self.tracks.iter().position(|t| t.path == track.path) {
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

    pub fn remove_track(&mut self, path: &Path) {
        self.tracks.retain(|t| t.path != path);
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

    #[test]
    fn backfill_fills_local_dates_from_file_mtime() {
        use super::{Album, Track};

        // Arrange — a real local file (so mtime exists) and a remote-style
        // path that does not, both lacking a stored `date_added`.
        let file = std::env::temp_dir().join(format!("kopuz-backfill-{}.mp3", std::process::id()));
        std::fs::write(&file, b"x").unwrap();
        let mut library = Library::default();
        library.tracks.push(Track {
            path: file.clone(),
            album_id: "local-album".into(),
            title: "Local".into(),
            artist: "A".into(),
            artists: vec![],
            album: "Album".into(),
            duration: 0,
            khz: 0,
            bitrate: 0,
            track_number: None,
            disc_number: None,
            musicbrainz_release_id: None,
            musicbrainz_recording_id: None,
            musicbrainz_track_id: None,
            playlist_item_id: None,
            date_added: None,
        });
        library.albums.push(Album {
            id: "local-album".into(),
            title: "Album".into(),
            artist: "A".into(),
            genre: String::new(),
            year: 0,
            cover_path: None,
            manual_cover: false,
            date_added: None,
        });

        // Act
        library.backfill_local_dates();

        // Assert — local track and its album both gained a date.
        assert!(library.tracks[0].date_added.is_some());
        assert_eq!(library.albums[0].date_added, library.tracks[0].date_added);

        let _ = std::fs::remove_file(&file);
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

impl PlaylistStore {
    #[tracing::instrument(name = "playlists.load", skip_all)]
    pub fn load(path: &Path) -> std::io::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let data = fs::read_to_string(path)?;
        let store: Self = serde_json::from_str(&data)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        tracing::debug!(
            bytes = data.len(),
            playlists = store.playlists.len(),
            "playlists loaded"
        );
        Ok(store)
    }

    #[tracing::instrument(name = "playlists.save", skip_all, fields(playlists = self.playlists.len()))]
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_string(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        tracing::debug!(bytes = data.len(), "writing playlists to disk");
        fs::write(path, data)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct FavoritesStore {
    #[serde(default)]
    pub local_favorites: Vec<PathBuf>,
    #[serde(default)]
    pub jellyfin_favorites: Vec<String>,
}

impl FavoritesStore {
    #[tracing::instrument(name = "favorites.load", skip_all)]
    pub fn load(path: &Path) -> std::io::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let data = fs::read_to_string(path)?;
        let store: Self = serde_json::from_str(&data)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        tracing::debug!(
            bytes = data.len(),
            local = store.local_favorites.len(),
            remote = store.jellyfin_favorites.len(),
            "favorites loaded"
        );
        Ok(store)
    }

    #[tracing::instrument(name = "favorites.save", skip_all)]
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        fs::write(path, data)
    }

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
