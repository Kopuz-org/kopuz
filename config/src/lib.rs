use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CustomTheme {
    pub name: String,
    pub vars: HashMap<String, String>,
}
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum MusicSource {
    #[default]
    Local,
    #[serde(alias = "Jellyfin")]
    Server,
}

impl MusicSource {
    pub fn is_server(&self) -> bool {
        matches!(self, Self::Server)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum MusicService {
    #[default]
    Jellyfin,
    #[serde(alias = "Navidrome")]
    Subsonic,
    Custom,
}

impl MusicService {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Jellyfin => "Jellyfin",
            Self::Subsonic => "Subsonic",
            Self::Custom => "Custom",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SortOrder {
    Title,
    Artist,
    Album,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub server: Option<MusicServer>,
    #[serde(default)]
    pub active_source: MusicSource,
    pub music_directory: PathBuf,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_device_id")]
    pub device_id: String,
    #[serde(default = "default_discord_presence")]
    pub discord_presence: Option<bool>,
    #[serde(default = "default_sort_order")]
    pub sort_order: SortOrder,
    #[serde(default)]
    pub listen_counts: HashMap<String, u64>,
    #[serde(default)]
    pub musicbrainz_token: String,
    #[serde(default)]
    pub lastfm_token: String,
    #[serde(default)]
    pub reduce_animations: bool,
    #[serde(default = "default_volume")]
    pub volume: f32,
    #[serde(default)]
    pub custom_themes: HashMap<String, CustomTheme>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MusicServer {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub service: MusicService,
    pub access_token: Option<String>,
    pub user_id: Option<String>,
}

pub type JellyfinServer = MusicServer;

impl MusicServer {
    pub fn new(name: String, url: String) -> Self {
        Self::new_with_service(name, url, MusicService::Jellyfin)
    }

    pub fn new_with_service(name: String, url: String, service: MusicService) -> Self {
        Self {
            name,
            // trim once here so every consumer gets a clean url to prevent broken links
            url: url.trim_end_matches('/').to_string(),
            service,
            access_token: None,
            user_id: None,
        }
    }
}

fn default_theme() -> String {
    "default".to_string()
}

fn default_device_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn default_discord_presence() -> Option<bool> {
    Some(true)
}

fn default_sort_order() -> SortOrder {
    SortOrder::Title
}

fn default_volume() -> f32 {
    1.0
}

impl Default for AppConfig {
    fn default() -> Self {
        let music_directory = directories::UserDirs::new()
            .and_then(|u| u.audio_dir().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("./assets"));
        Self {
            server: None,
            active_source: MusicSource::Local,
            music_directory,
            theme: default_theme(),
            device_id: default_device_id(),
            discord_presence: Some(true),
            sort_order: default_sort_order(),
            listen_counts: HashMap::new(),
            musicbrainz_token: String::new(),
            lastfm_token: String::new(),
            reduce_animations: false,
            volume: default_volume(),
            custom_themes: HashMap::new(),
        }
    }
}

impl Default for MusicServer {
    fn default() -> Self {
        Self {
            name: String::new(),
            url: String::new(),
            service: MusicService::Jellyfin,
            access_token: None,
            user_id: None,
        }
    }
}

impl AppConfig {
    pub fn active_service(&self) -> Option<MusicService> {
        if self.active_source.is_server() {
            self.server.as_ref().map(|server| server.service)
        } else {
            None
        }
    }

    pub fn uses_jellyfin_server(&self) -> bool {
        self.active_service() == Some(MusicService::Jellyfin)
    }

    pub fn load(path: &Path) -> Self {
        if !path.exists() {
            return Self::default();
        }
        match fs::read_to_string(path) {
            Ok(data) => match serde_json::from_str::<Self>(&data) {
                Ok(config) => config,
                Err(e) => {
                    eprintln!("Failed to parse config at {:?}: {}", path, e);
                    Self::default()
                }
            },
            Err(e) => {
                eprintln!("Failed to read config at {:?}: {}", path, e);
                Self::default()
            }
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                eprintln!("Failed to create config directory {:?}: {}", parent, e);
                return Err(e);
            }
        }
        let data = match serde_json::to_string_pretty(self) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("Failed to serialize config: {}", e);
                return Err(std::io::Error::new(std::io::ErrorKind::Other, e));
            }
        };
        if let Err(e) = fs::write(path, data) {
            eprintln!("Failed to write config to {:?}: {}", path, e);
            return Err(e);
        }
        Ok(())
    }
}
