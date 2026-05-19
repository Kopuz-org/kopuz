use crate::manifest::{ManifestError, StationManifest};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryIndex {
    pub registry_name: String,
    pub description: String,
    pub stations: Vec<RegistryStationRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryStationRef {
    pub id: String,
    pub manifest_url: String,
}

#[derive(Debug, Default)]
pub struct StationRegistry {
    stations: HashMap<String, StationManifest>,
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON parsing error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Manifest validation error: {0}")]
    Validation(#[from] ManifestError),
    #[error("Network error: {0}")]
    Network(String),
    #[error("Invalid URL or path: {0}")]
    InvalidUrl(String),
}

impl StationRegistry {
    pub fn new() -> Self {
        Self {
            stations: HashMap::new(),
        }
    }

    pub async fn import_registry(&mut self, url_or_path: &str) -> Result<(), RegistryError> {
        let (index_content, base_url_or_dir) = if url_or_path.starts_with("http://") || url_or_path.starts_with("https://") {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let resp = reqwest::get(url_or_path)
                    .await
                    .map_err(|e| RegistryError::Network(e.to_string()))?;
                let text = resp
                    .text()
                    .await
                    .map_err(|e| RegistryError::Network(e.to_string()))?;

                // Determine base URL by removing the filename
                let base_url = if let Some(idx) = url_or_path.rfind('/') {
                    &url_or_path[..idx]
                } else {
                    url_or_path
                };
                (text, base_url.to_string())
            }
            #[cfg(target_arch = "wasm32")]
            {
                return Err(RegistryError::Network("HTTP fetching not supported on WASM yet".into()));
            }
        } else {
            // Local file path
            let path = Path::new(url_or_path);
            let text = std::fs::read_to_string(path)?;
            let parent = path.parent().unwrap_or(Path::new("")).to_string_lossy().to_string();
            (text, parent)
        };

        let index: RegistryIndex = serde_json::from_str(&index_content)?;

        for station_ref in index.stations {
            let manifest_content = if station_ref.manifest_url.starts_with("http://") || station_ref.manifest_url.starts_with("https://") {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    reqwest::get(&station_ref.manifest_url)
                        .await
                        .map_err(|e| RegistryError::Network(e.to_string()))?
                        .text()
                        .await
                        .map_err(|e| RegistryError::Network(e.to_string()))?
                }
                #[cfg(target_arch = "wasm32")]
                {
                    String::new()
                }
            } else if base_url_or_dir.starts_with("http://") || base_url_or_dir.starts_with("https://") {
                // Resolve relative URL
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let url = format!("{}/{}", base_url_or_dir, station_ref.manifest_url.trim_start_matches("./"));
                    reqwest::get(&url)
                        .await
                        .map_err(|e| RegistryError::Network(e.to_string()))?
                        .text()
                        .await
                        .map_err(|e| RegistryError::Network(e.to_string()))?
                }
                #[cfg(target_arch = "wasm32")]
                {
                    String::new()
                }
            } else {
                // Local relative path
                let mut path = PathBuf::from(&base_url_or_dir);
                path.push(station_ref.manifest_url.trim_start_matches("./"));
                std::fs::read_to_string(path)?
            };

            if let Ok(manifest) = serde_json::from_str::<StationManifest>(&manifest_content) {
                if manifest.validate().is_ok() {
                    self.stations.insert(manifest.id.clone(), manifest);
                } else {
                    tracing::warn!("Imported manifest {} failed validation", station_ref.id);
                }
            } else {
                tracing::warn!("Failed to parse manifest for {}", station_ref.id);
            }
        }

        Ok(())
    }

    pub fn all_stations(&self) -> Vec<&StationManifest> {
        let mut vec: Vec<_> = self.stations.values().collect();
        vec.sort_by(|a, b| a.name.cmp(&b.name));
        vec
    }

    pub fn get(&self, id: &str) -> Option<&StationManifest> {
        self.stations.get(id)
    }

    pub fn create_provider(&self, station_id: &str) -> Option<crate::provider::DynamicProvider> {
        let manifest = self.get(station_id)?;
        Some(crate::provider::DynamicProvider::new(manifest.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_import_local_registry() {
        let dir = tempdir().unwrap();
        let index_path = dir.path().join("index.json");
        let manifest_path = dir.path().join("test_station.json");

        let index_json = r#"{
            "registry_name": "Test Registry",
            "description": "Test",
            "stations": [
                {
                    "id": "test_station",
                    "manifest_url": "./test_station.json"
                }
            ]
        }"#;

        let manifest_json = r#"{
            "schema_version": "1.0",
            "id": "test_station",
            "name": "Test Station",
            "description": "Test",
            "streams": [
                {
                    "id": "main",
                    "name": "Main",
                    "url": "https://example.com/stream"
                }
            ]
        }"#;

        fs::write(&index_path, index_json).unwrap();
        fs::write(&manifest_path, manifest_json).unwrap();

        let mut registry = StationRegistry::new();
        registry.import_registry(index_path.to_str().unwrap()).await.unwrap();

        assert_eq!(registry.stations.len(), 1);
        assert!(registry.get("test_station").is_some());
    }
}
