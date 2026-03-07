use serde::{Deserialize, Serialize};
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
    pub bitrate: u8,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    #[serde(default)]
    pub musicbrainz_release_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct Library {
    #[serde(default)]
    pub root_path: PathBuf,
    pub tracks: Vec<Track>,
    pub albums: Vec<Album>,
    #[serde(default)]
    pub jellyfin_tracks: Vec<Track>,
    #[serde(default)]
    pub jellyfin_albums: Vec<Album>,
    #[serde(default)]
    pub jellyfin_genres: Vec<(String, String)>, // (Name, ID)
}

impl Library {
    pub fn new(root_path: PathBuf) -> Self {
        Self {
            root_path,
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
        let data = serde_json::to_string_pretty(self)
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub tracks: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JellyfinPlaylist {
    pub id: String,
    pub name: String,
    pub tracks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct PlaylistStore {
    pub playlists: Vec<Playlist>,
    #[serde(default)]
    pub jellyfin_playlists: Vec<JellyfinPlaylist>,
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
        let data = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        fs::write(path, data)
    }
}
