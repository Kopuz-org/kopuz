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
    pub playlist_item_id: Option<String>,
    #[serde(default)]
    pub artists: Vec<String>,
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
    #[serde(default)]
    pub server_artist_images: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub local_artist_images: std::collections::HashMap<String, PathBuf>,
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

    pub fn load(path: &Path) -> std::io::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let data = fs::read_to_string(path)?;
        let library = serde_json::from_str(&data)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        Ok(library)
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_string(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
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
    use super::{
        Album, FavoritesStore, JellyfinPlaylist, Library, Playlist, PlaylistFolder, PlaylistStore,
        Track,
    };
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_file(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("kopuz_reader_{name}_{unique}.json"))
    }

    fn temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("kopuz_reader_{name}_{unique}"));
        fs::create_dir(&dir).unwrap();
        dir
    }

    #[test]
    fn library_deserializes_modern_root_paths() {
        let json = r#"{
            "root_paths": ["/music1", "/music2"],
            "tracks": [],
            "albums": []
        }"#;

        let library: Library = serde_json::from_str(json).unwrap();

        assert_eq!(
            library.root_paths,
            vec![PathBuf::from("/music1"), PathBuf::from("/music2")]
        );
    }

    #[test]
    fn library_deserializes_legacy_single_root_path_alias() {
        let json = r#"{
            "root_path": "/music",
            "tracks": [],
            "albums": []
        }"#;

        let library: Library = serde_json::from_str(json).unwrap();

        assert_eq!(library.root_paths, vec![PathBuf::from("/music")]);
    }

    #[test]
    fn sample_track_has_expected_defaults() {
        let track = sample_track("/music/track.flac", "alb_one", "Title");
        assert_eq!(track.path, PathBuf::from("/music/track.flac"));
        assert_eq!(track.album_id, "alb_one");
    }

    #[test]
    fn library_load_non_existent_file_returns_default() {
        let dir = temp_dir("library_missing");
        let path = dir.join("library.json");
        let library = Library::load(&path).unwrap();
        assert!(library.root_paths.is_empty());
        assert!(library.tracks.is_empty());
        let _ = fs::remove_dir(dir);
    }

    #[test]
    fn library_load_invalid_json_returns_error() {
        let path = temp_file("library_invalid");
        fs::write(&path, "{not valid json").unwrap();

        let result = Library::load(&path);

        assert!(result.is_err());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn library_add_track_handles_empty() {
        let mut library = Library::default();
        let track = sample_track("/music/track.flac", "alb_one", "Old Title");
        library.add_track(track.clone());
        assert_eq!(library.tracks.len(), 1);
        assert_eq!(library.tracks[0], track);
    }

    #[test]
    fn library_remove_track_removes_by_path() {
        let mut library = Library::default();
        library.add_track(sample_track("/music/one.flac", "alb_one", "One"));
        library.add_track(sample_track("/music/two.flac", "alb_two", "Two"));

        library.remove_track(Path::new("/music/one.flac"));

        assert_eq!(library.tracks.len(), 1);
        assert_eq!(library.tracks[0].path, PathBuf::from("/music/two.flac"));

        // Remove non existent
        library.remove_track(Path::new("/music/three.flac"));
        assert_eq!(library.tracks.len(), 1);
    }

    fn sample_track(path: &str, album_id: &str, title: &str) -> Track {
        Track {
            path: PathBuf::from(path),
            album_id: album_id.to_string(),
            title: title.to_string(),
            artist: "Artist".to_string(),
            album: "Album".to_string(),
            duration: 180,
            khz: 44_100,
            bitrate: 320,
            track_number: Some(1),
            disc_number: Some(1),
            musicbrainz_release_id: None,
            playlist_item_id: None,
            artists: vec!["Artist".to_string()],
        }
    }

    #[test]
    fn add_track_replaces_existing_track_by_path() {
        let mut library = Library::default();
        library.add_track(sample_track("/music/track.flac", "alb_one", "Old Title"));
        library.add_track(sample_track("/music/track.flac", "alb_two", "New Title"));

        assert_eq!(library.tracks.len(), 1);
        assert_eq!(library.tracks[0].album_id, "alb_two");
        assert_eq!(library.tracks[0].title, "New Title");
    }

    #[test]
    fn library_save_and_load_round_trip_preserves_artist_images_and_tracks() {
        let path = temp_file("library_round_trip");
        let mut library = Library::new(vec![PathBuf::from("/music")]);
        library
            .local_artist_images
            .insert("alohaii".to_string(), PathBuf::from("/covers/alohaii.jpg"));
        library.server_artist_images.insert(
            "alohaii".to_string(),
            "https://example.com/alohaii.jpg".to_string(),
        );
        library.add_album(Album {
            id: "alb_patchwork".to_string(),
            title: "Patchwork".to_string(),
            artist: "Alohalii".to_string(),
            genre: "Pop".to_string(),
            year: 2025,
            cover_path: Some(PathBuf::from("/covers/patchwork.png")),
            manual_cover: false,
        });
        library.add_track(sample_track(
            "/music/track.flac",
            "alb_patchwork",
            "Safe Room",
        ));

        library.save(&path).unwrap();
        let loaded = Library::load(&path).unwrap();

        assert_eq!(loaded.root_paths, vec![PathBuf::from("/music")]);
        assert_eq!(
            loaded.local_artist_images.get("alohaii"),
            Some(&PathBuf::from("/covers/alohaii.jpg"))
        );
        assert_eq!(
            loaded.server_artist_images.get("alohaii"),
            Some(&"https://example.com/alohaii.jpg".to_string())
        );
        assert_eq!(loaded.tracks.len(), 1);
        assert_eq!(loaded.albums.len(), 1);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn add_album_preserves_existing_cover_when_new_cover_is_missing() {
        let mut library = Library::default();
        library.add_album(Album {
            id: "alb_patchwork".to_string(),
            title: "Patchwork".to_string(),
            artist: "Alohalii".to_string(),
            genre: "Pop".to_string(),
            year: 2025,
            cover_path: Some(PathBuf::from("/covers/patchwork.png")),
            manual_cover: false,
        });

        library.add_album(Album {
            id: "alb_patchwork".to_string(),
            title: "Patchwork".to_string(),
            artist: "Alohalii".to_string(),
            genre: "Dream Pop".to_string(),
            year: 2026,
            cover_path: None,
            manual_cover: false,
        });

        assert_eq!(library.albums.len(), 1);
        assert_eq!(
            library.albums[0].cover_path,
            Some(PathBuf::from("/covers/patchwork.png"))
        );
        assert_eq!(library.albums[0].genre, "Dream Pop");
        assert_eq!(library.albums[0].year, 2026);
    }

    #[test]
    fn add_album_replaces_existing_cover_when_new_cover_is_provided() {
        let mut library = Library::default();
        library.add_album(Album {
            id: "alb_patchwork".to_string(),
            title: "Patchwork".to_string(),
            artist: "Alohalii".to_string(),
            genre: "Pop".to_string(),
            year: 2025,
            cover_path: Some(PathBuf::from("/covers/old.png")),
            manual_cover: false,
        });

        library.add_album(Album {
            id: "alb_patchwork".to_string(),
            title: "Patchwork".to_string(),
            artist: "Alohalii".to_string(),
            genre: "Dream Pop".to_string(),
            year: 2026,
            cover_path: Some(PathBuf::from("/covers/new.png")),
            manual_cover: false,
        });

        assert_eq!(library.albums.len(), 1);
        assert_eq!(
            library.albums[0].cover_path,
            Some(PathBuf::from("/covers/new.png"))
        );
    }

    #[test]
    fn remove_album_removes_album_and_associated_tracks() {
        let mut library = Library::default();
        library.add_album(Album {
            id: "alb_one".to_string(),
            title: "One".to_string(),
            artist: "Artist".to_string(),
            genre: "Rock".to_string(),
            year: 2024,
            cover_path: None,
            manual_cover: false,
        });
        library.add_album(Album {
            id: "alb_two".to_string(),
            title: "Two".to_string(),
            artist: "Artist".to_string(),
            genre: "Rock".to_string(),
            year: 2025,
            cover_path: None,
            manual_cover: false,
        });
        library.add_track(sample_track("/music/one.flac", "alb_one", "One"));
        library.add_track(sample_track("/music/two.flac", "alb_two", "Two"));

        library.remove_album("alb_one");

        assert_eq!(library.albums.len(), 1);
        assert_eq!(library.albums[0].id, "alb_two");
        assert_eq!(library.tracks.len(), 1);
        assert_eq!(library.tracks[0].album_id, "alb_two");
    }

    #[test]
    fn playlist_store_load_missing_file_returns_default() {
        let store = PlaylistStore::load(Path::new("not_a_real_playlist_store.json")).unwrap();
        assert!(store.playlists.is_empty());
        assert!(store.jellyfin_playlists.is_empty());
        assert!(store.folders.is_empty());
    }

    #[test]
    fn playlist_store_round_trips_playlists_folders_and_server_playlists() {
        let path = temp_file("playlist_store_round_trip");
        let store = PlaylistStore {
            playlists: vec![Playlist {
                id: "local-1".to_string(),
                name: "Morning".to_string(),
                tracks: vec![PathBuf::from("/music/song.flac")],
                cover_path: Some(PathBuf::from("/covers/morning.png")),
            }],
            jellyfin_playlists: vec![JellyfinPlaylist {
                id: "srv-1".to_string(),
                name: "Server Mix".to_string(),
                tracks: vec!["track-a".to_string(), "track-b".to_string()],
                image_tag: Some("abc123".to_string()),
                cover_path: Some(PathBuf::from("/covers/server.png")),
            }],
            folders: vec![PlaylistFolder {
                id: "folder-1".to_string(),
                name: "Favorites".to_string(),
                playlist_ids: vec!["local-1".to_string(), "srv-1".to_string()],
            }],
        };

        store.save(&path).unwrap();
        let loaded = PlaylistStore::load(&path).unwrap();

        assert_eq!(loaded, store);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn toggle_local_adds_then_removes_same_path() {
        let mut favorites = FavoritesStore::default();
        let path = PathBuf::from("/music/song.flac");

        assert!(favorites.toggle_local(path.clone()));
        assert!(favorites.is_local_favorite(&path));
        assert!(!favorites.toggle_local(path.clone()));
        assert!(!favorites.is_local_favorite(&path));
    }

    #[test]
    fn set_jellyfin_adds_once_and_removes_cleanly() {
        let mut favorites = FavoritesStore::default();

        favorites.set_jellyfin("track-1".to_string(), true);
        favorites.set_jellyfin("track-1".to_string(), true);

        assert_eq!(favorites.jellyfin_favorites, vec!["track-1".to_string()]);
        assert!(favorites.is_jellyfin_favorite("track-1"));

        favorites.set_jellyfin("track-1".to_string(), false);

        assert!(!favorites.is_jellyfin_favorite("track-1"));
        assert!(favorites.jellyfin_favorites.is_empty());
    }

    #[test]
    fn favorites_store_round_trips_local_and_server_favorites() {
        let path = temp_file("favorites_store_round_trip");
        let store = FavoritesStore {
            local_favorites: vec![PathBuf::from("/music/song.flac")],
            jellyfin_favorites: vec!["track-1".to_string(), "track-2".to_string()],
        };

        store.save(&path).unwrap();
        let loaded = FavoritesStore::load(&path).unwrap();

        assert_eq!(loaded, store);
        let _ = fs::remove_file(path);
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
    pub fn load(path: &Path) -> std::io::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let data = fs::read_to_string(path)?;
        let store = serde_json::from_str(&data)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        Ok(store)
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_string(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
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
    pub fn load(path: &Path) -> std::io::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let data = fs::read_to_string(path)?;
        let store = serde_json::from_str(&data)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        Ok(store)
    }

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
