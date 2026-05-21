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
pub enum IndexError {
    #[error("Registry name cannot be empty")]
    EmptyName,
    #[error("Registry description cannot be empty")]
    EmptyDescription,
    #[error("Registry must contain at least one station")]
    NoStations,
    #[error("Station ID must contain only alphanumeric characters, underscores, and dashes: {0}")]
    InvalidStationId(String),
    #[error("Manifest URL cannot be empty for station: {0}")]
    InvalidManifestUrl(String),
}

impl RegistryIndex {
    pub fn validate(&self) -> Result<(), IndexError> {
        if self.registry_name.trim().is_empty() {
            return Err(IndexError::EmptyName);
        }
        if self.description.trim().is_empty() {
            return Err(IndexError::EmptyDescription);
        }
        if self.stations.is_empty() {
            return Err(IndexError::NoStations);
        }
        for station in &self.stations {
            if station.id.trim().is_empty()
                || !station
                    .id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                return Err(IndexError::InvalidStationId(station.id.clone()));
            }
            if station.manifest_url.trim().is_empty() {
                return Err(IndexError::InvalidManifestUrl(station.id.clone()));
            }
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON parsing error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Registry index validation error: {0}")]
    InvalidIndex(#[from] IndexError),
    #[error("Manifest validation error: {0}")]
    Validation(#[from] ManifestError),
    #[error("Network error: {0}")]
    Network(String),
    #[error("Invalid URL or path: {0}")]
    InvalidUrl(String),
    #[error("Station errors: {0}")]
    StationErrors(String),
}

async fn fetch_content(url_or_path: &str, base_url_or_dir: Option<&str>) -> Result<(String, String), RegistryError> {
    let is_http = |s: &str| s.starts_with("http://") || s.starts_with("https://");

    if is_http(url_or_path) || base_url_or_dir.map_or(false, is_http) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let url = if is_http(url_or_path) {
                url_or_path.to_string()
            } else {
                format!("{}/{}", base_url_or_dir.unwrap(), url_or_path.trim_start_matches("./"))
            };

            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .map_err(|e| RegistryError::Network(e.to_string()))?;

            let text = client.get(&url)
                .send()
                .await
                .map_err(|e| RegistryError::Network(e.to_string()))?
                .text()
                .await
                .map_err(|e| RegistryError::Network(e.to_string()))?;

            let new_base = if let Some(idx) = url.rfind('/') {
                url[..idx].to_string()
            } else {
                url
            };

            Ok((text, new_base))
        }
        #[cfg(target_arch = "wasm32")]
        {
            Err(RegistryError::Network("HTTP fetching not supported on WASM yet".into()))
        }
    } else {
        let path = if let Some(base) = base_url_or_dir {
            let mut p = PathBuf::from(base);
            p.push(url_or_path.trim_start_matches("./"));
            p
        } else {
            PathBuf::from(url_or_path)
        };

        let text = std::fs::read_to_string(&path)?;
        let parent = path.parent().unwrap_or(Path::new("")).to_string_lossy().to_string();

        Ok((text, parent))
    }
}

impl StationRegistry {
    pub fn new() -> Self {
        Self {
            stations: HashMap::new(),
        }
    }

    pub async fn import_registry(&mut self, url_or_path: &str) -> Result<(), RegistryError> {
        let (index_content, base_url_or_dir) = match fetch_content(url_or_path, None).await {
            Ok(res) => res,
            Err(e) => {
                tracing::error!("Failed to fetch registry index from {}: {}", url_or_path, e);
                return Err(e);
            }
        };

        let index: RegistryIndex = match serde_json::from_str(&index_content) {
            Ok(idx) => idx,
            Err(e) => {
                tracing::error!("Failed to parse registry index from {}: {}", url_or_path, e);
                return Err(RegistryError::Json(e));
            }
        };

        if let Err(e) = index.validate() {
            tracing::error!("Invalid registry index from {}: {}", url_or_path, e);
            return Err(RegistryError::InvalidIndex(e));
        }

        let mut station_errors = Vec::new();

        for station_ref in index.stations {
            match fetch_content(&station_ref.manifest_url, Some(&base_url_or_dir)).await {
                Ok((manifest_content, _)) => {
                    match serde_json::from_str::<StationManifest>(&manifest_content) {
                        Ok(manifest) => {
                            if manifest.id != station_ref.id {
                                let msg = format!("Station id mismatch: index id={} manifest id={}", station_ref.id, manifest.id);
                                tracing::warn!("{}", msg);
                                station_errors.push(msg);
                                continue;
                            }
                            if let Err(e) = manifest.validate() {
                                let msg = format!("Imported station '{}' failed validation: {}", station_ref.id, e);
                                tracing::warn!("{}", msg);
                                station_errors.push(msg);
                            } else {
                                self.stations.insert(manifest.id.clone(), manifest);
                            }
                        }
                        Err(e) => {
                            let msg = format!("Failed to parse station for '{}': {}", station_ref.id, e);
                            tracing::warn!("{}", msg);
                            station_errors.push(msg);
                        }
                    }
                }
                Err(e) => {
                    let msg = format!("Failed to fetch station '{}': {}", station_ref.id, e);
                    tracing::warn!("{}", msg);
                    station_errors.push(msg);
                }
            }
        }

        if !station_errors.is_empty() {
            let mut combined = station_errors[0].clone();
            if station_errors.len() > 1 {
                combined.push_str(&format!(" (and {} more errors)", station_errors.len() - 1));
            }
            return Err(RegistryError::StationErrors(combined));
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

    #[test]
    fn test_validate_registry_index() {
        let valid_json = r#"{
            "registry_name": "Official Kopuz Radio Registry",
            "description": "Default radio stations",
            "stations": [
                { "id": "listen_moe", "manifest_url": "./stations/listen_moe.json" }
            ]
        }"#;
        let index: RegistryIndex = serde_json::from_str(valid_json).unwrap();
        assert!(index.validate().is_ok());

        let invalid_json_empty_name = r#"{
            "registry_name": "   ",
            "description": "desc",
            "stations": [{ "id": "a", "manifest_url": "url" }]
        }"#;
        let index: RegistryIndex = serde_json::from_str(invalid_json_empty_name).unwrap();
        assert!(matches!(index.validate(), Err(IndexError::EmptyName)));

        let invalid_json_no_stations = r#"{
            "registry_name": "name",
            "description": "desc",
            "stations": []
        }"#;
        let index: RegistryIndex = serde_json::from_str(invalid_json_no_stations).unwrap();
        assert!(matches!(index.validate(), Err(IndexError::NoStations)));

        let invalid_json_empty_id = r#"{
            "registry_name": "name",
            "description": "desc",
            "stations": [{ "id": "", "manifest_url": "url" }]
        }"#;
        let index: RegistryIndex = serde_json::from_str(invalid_json_empty_id).unwrap();
        assert!(matches!(index.validate(), Err(IndexError::InvalidStationId(_))));
    }
}
