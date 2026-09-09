//! Internet radio: the station registry, station search, and pins.
//!
//! The registry used to be built in the UI process, which meant importing
//! registry URLs, unwrapping stream playlists and holding station manifests
//! all happened in a frontend -- and only in the one frontend that did it.
//! It lives here now, rebuilt from config whenever the registry list or the
//! pins change, and the session plays a station by id.

use std::sync::Arc;

use api::{ApiError, ArtworkTarget, ErrorCode, RadioStationInfo, RadioStreamInfo};
use radio::manifest::{MetadataSourceDef, StationManifest};
use radio::registry::StationRegistry;
use tokio::sync::{RwLock, watch};

use crate::config_service::ConfigService;
use crate::library::LibraryService;

pub struct RadioService {
    registry: RwLock<StationRegistry>,
    config: Arc<ConfigService>,
    library: Arc<LibraryService>,
}

/// A station's own picture, where its metadata carries one.
fn manifest_artwork(manifest: &StationManifest) -> Option<String> {
    match manifest.metadata.as_ref() {
        Some(MetadataSourceDef::Static(metadata)) => metadata.cover_url.clone(),
        _ => None,
    }
}

/// The wire row. `name` and `description` stay as the registry authored them
/// -- often translation keys -- because only the client knows the locale.
fn station_info(manifest: &StationManifest, pinned: bool) -> RadioStationInfo {
    RadioStationInfo {
        artwork: manifest_artwork(manifest)
            .map(|url| crate::artwork::url_ref(ArtworkTarget::Station(manifest.id.clone()), &url)),
        id: manifest.id.clone(),
        name: manifest.name.clone(),
        description: manifest.description.clone(),
        tags: manifest.tags.clone(),
        streams: manifest
            .streams
            .iter()
            .map(|stream| RadioStreamInfo {
                id: stream.id.clone(),
                name: stream.name.clone(),
                icon: stream.icon.clone(),
            })
            .collect(),
        pinned,
        icon: manifest.icon.clone(),
    }
}

impl RadioService {
    pub fn new(config: Arc<ConfigService>, library: Arc<LibraryService>) -> Arc<Self> {
        Arc::new(Self {
            registry: RwLock::new(StationRegistry::new()),
            config,
            library,
        })
    }

    /// Rebuild the registry from the configured sources. Called at boot and
    /// whenever the registry list or the pins change, so a settings toggle
    /// takes effect without a restart.
    pub async fn reload(&self) -> Result<(), ApiError> {
        let config = self.config.view().await?.config;
        let mut registry = StationRegistry::new();
        for entry in config.radio_registries.iter().filter(|entry| entry.enabled) {
            if let Err(error) = registry.import_registry(&entry.url).await {
                tracing::warn!(url = %entry.url, %error, "radio registry import failed");
            }
        }
        for json in &config.pinned_stations {
            match serde_json::from_str(json) {
                Ok(manifest) => registry.pin_manifest(manifest),
                Err(error) => tracing::warn!(%error, "pinned radio station is invalid"),
            }
        }
        self.publish(registry).await;
        Ok(())
    }

    /// Rebuild whenever the configured registries change, so a settings toggle
    /// takes effect without a restart. Pins reload themselves through `pin`,
    /// which is why the key ignores them: re-importing every registry for a
    /// pin would cost a round of network for nothing.
    pub fn watch_config(self: &Arc<Self>, mut config: watch::Receiver<config::AppConfig>) {
        let service = self.clone();
        tokio::spawn(async move {
            let mut current = registry_key(&config.borrow());
            while config.changed().await.is_ok() {
                let next = registry_key(&config.borrow());
                if next == current {
                    continue;
                }
                current = next;
                if let Err(error) = service.reload().await {
                    tracing::warn!(%error, "radio registry reload failed");
                }
            }
        });
    }

    /// Share the registry with whatever resolves a stream at play time.
    async fn publish(&self, registry: StationRegistry) {
        let snapshot = Arc::new(registry.clone());
        *self.registry.write().await = registry;
        self.library.set_station_registry(snapshot);
    }

    pub async fn stations(&self) -> Vec<RadioStationInfo> {
        let registry = self.registry.read().await;
        registry
            .all_stations()
            .into_iter()
            .map(|station| {
                let pinned = registry.is_registry_station(&station.id);
                station_info(station, pinned)
            })
            .collect()
    }

    /// The station's icon URL, for the artwork service to proxy.
    pub async fn artwork_url(&self, id: &str) -> Option<String> {
        self.registry
            .read()
            .await
            .get(id)
            .and_then(manifest_artwork)
    }

    /// Search the public directory. Hits join the live registry before they
    /// are returned, so playing one by id immediately afterwards works.
    pub async fn search(&self, query: &str, limit: u32) -> Result<Vec<RadioStationInfo>, ApiError> {
        let stations = if query.trim().is_empty() {
            radio::browser::top_stations(limit).await
        } else {
            radio::browser::search(query, limit).await
        }
        .map_err(|error| ApiError::new(ErrorCode::SourceUnreachable, error.to_string()))?;

        let mut registry = self.registry.write().await;
        let mut found = Vec::with_capacity(stations.len());
        for station in stations {
            let manifest = radio::browser::to_manifest(&station);
            let known = registry.get(&manifest.id).is_some();
            let pinned = registry.is_registry_station(&manifest.id);
            found.push(station_info(&manifest, pinned));
            // Re-inserting a station the registry already holds would demote a
            // pinned one back to a runtime entry, so a search would silently
            // unpin whatever it happened to return.
            if !known {
                registry.insert_manifest(manifest);
            }
        }
        let snapshot = Arc::new(registry.clone());
        drop(registry);
        self.library.set_station_registry(snapshot);
        Ok(found)
    }

    /// Import a registry into a throwaway of its own, so a client can tell a
    /// user their URL is wrong before it writes it into the config that this
    /// service rebuilds from.
    pub async fn validate_registry(&self, url: &str) -> Result<u32, ApiError> {
        let mut registry = StationRegistry::new();
        registry
            .import_registry(url)
            .await
            .map_err(|error| ApiError::invalid_input(error.to_string()))?;
        Ok(registry.all_stations().len() as u32)
    }

    /// Pin a station, which persists its manifest in config so it survives a
    /// registry that stops listing it.
    pub async fn pin(&self, id: &str, pinned: bool) -> Result<(), ApiError> {
        let manifest = self
            .registry
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| ApiError::not_found("no such radio station"))?;
        manifest
            .validate()
            .map_err(|error| ApiError::invalid_input(error.to_string()))?;
        let json = serde_json::to_string(&manifest)
            .map_err(|error| ApiError::internal(error.to_string()))?;

        let mut config = self.config.view().await?.config;
        config.pinned_stations.retain(|existing| {
            serde_json::from_str::<StationManifest>(existing)
                .map(|station| station.id != manifest.id)
                .unwrap_or(true)
        });
        if pinned {
            config.pinned_stations.push(json);
        }
        self.config.set(config).await?;
        self.reload().await?;
        if !pinned {
            // Unpinning demotes rather than forgets: the station drops out of
            // the selected list but whatever is playing it keeps working.
            let mut registry = self.registry.write().await;
            registry.insert_manifest(manifest);
            let snapshot = Arc::new(registry.clone());
            drop(registry);
            self.library.set_station_registry(snapshot);
        }
        Ok(())
    }
}

/// What the registry is built from: the enabled registry URLs, in order.
fn registry_key(config: &config::AppConfig) -> String {
    config
        .radio_registries
        .iter()
        .filter(|entry| entry.enabled)
        .map(|entry| entry.url.as_str())
        .collect::<Vec<_>>()
        .join(",")
}
