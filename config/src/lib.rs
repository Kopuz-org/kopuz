use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub server: Option<JellyfinServer>,
    pub music_directory: PathBuf,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_device_id")]
    pub device_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JellyfinServer {
    pub name: String,
    pub url: String,
    pub access_token: Option<String>,
}

fn default_theme() -> String {
    "default".to_string()
}

fn default_device_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: None,
            music_directory: PathBuf::from("./assets"),
            theme: default_theme(),
            device_id: default_device_id(),
        }
    }
}

impl Default for JellyfinServer {
    fn default() -> Self {
        Self {
            name: String::new(),
            url: String::new(),
            access_token: None,
        }
    }
}

impl AppConfig {
    pub fn load(path: &Path) -> Self {
        if !path.exists() {
            return Self::default();
        }
        match fs::read_to_string(path) {
            Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_string_pretty(self)?;
        fs::write(path, data)
    }
}
