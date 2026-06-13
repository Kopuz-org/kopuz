use reader::Track;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

fn default_queue_state_version() -> u8 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PersistedQueueState {
    #[serde(default = "default_queue_state_version")]
    pub version: u8,
    #[serde(default)]
    pub queue: Vec<Track>,
    #[serde(default)]
    pub current_queue_index: usize,
    #[serde(default)]
    pub progress_secs: u64,
    #[serde(default)]
    pub shuffle_order: Vec<usize>,
    #[serde(default)]
    pub shuffle_enabled: bool,
}

impl Default for PersistedQueueState {
    fn default() -> Self {
        Self {
            version: default_queue_state_version(),
            queue: Vec::new(),
            current_queue_index: 0,
            progress_secs: 0,
            shuffle_order: Vec::new(),
            shuffle_enabled: false,
        }
    }
}

impl PersistedQueueState {
    pub fn load(path: &Path) -> std::io::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let data = fs::read_to_string(path)?;
        let state = serde_json::from_str(&data)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        Ok(state)
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
