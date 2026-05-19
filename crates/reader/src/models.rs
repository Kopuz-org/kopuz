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
            if new_album.cover_path.is_none() {
                new_album.cover_path = self.albums[index].cover_path.clone();
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
