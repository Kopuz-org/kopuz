use super::*;

impl FrontendService {
    fn station_info(
        manifest: &radio::manifest::StationManifest,
        pinned: bool,
    ) -> api::RadioStationInfo {
        api::RadioStationInfo {
            id: manifest.id.clone(),
            name: manifest.name.clone(),
            description: manifest.description.clone(),
            icon: manifest.icon.clone(),
            artwork: match manifest.metadata.as_ref() {
                Some(radio::manifest::MetadataSourceDef::Static(metadata)) => {
                    metadata.cover_url.clone()
                }
                _ => None,
            },
            tags: manifest.tags.clone(),
            streams: manifest
                .streams
                .iter()
                .map(|stream| api::RadioStreamInfo {
                    id: stream.id.clone(),
                    name: stream.name.clone(),
                    url: stream.url.clone(),
                    icon: stream.icon.clone(),
                })
                .collect(),
            pinned,
        }
    }

    pub async fn reload_radio(&self) -> Result<(), ApiError> {
        let config = self.current().await;
        let mut registry = radio::registry::StationRegistry::new();
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
        *self.registry.write().await = registry.clone();
        self.library.set_station_registry(Arc::new(registry));
        Ok(())
    }

    pub async fn adopt_radio_registry(&self, registry: radio::registry::StationRegistry) {
        *self.registry.write().await = registry.clone();
        self.library.set_station_registry(Arc::new(registry));
    }

    pub async fn radio_stations(&self) -> Vec<api::RadioStationInfo> {
        let registry = self.registry.read().await;
        registry
            .all_stations()
            .into_iter()
            .map(|station| {
                let pinned = registry.is_registry_station(&station.id);
                Self::station_info(station, pinned)
            })
            .collect()
    }

    pub async fn track_radio(&self, key: &str) -> Result<Vec<api::TrackInfo>, ApiError> {
        if key.trim().is_empty() {
            return Err(ApiError::invalid_input("radio seed key is required"));
        }
        let config = self.current().await;
        let mut tracks = self
            .source()
            .await
            .start_radio(key)
            .await
            .map_err(Self::source_error)?;
        if !tracks.is_empty() {
            let seed = if tracks.iter().any(|track| track.id.key().as_ref() == key) {
                None
            } else {
                Some(
                    self.db
                        .tracks_by_keys(&config.active_source, &[key.to_string()])
                        .await
                        .map_err(Self::db_error)?
                        .into_iter()
                        .next()
                        .or_else(|| self.library.transient_track(key))
                        .ok_or_else(|| ApiError::not_found("radio seed track not found"))?,
                )
            };
            tracks = Self::pin_radio_seed(key, seed, tracks);
        }
        self.library.register_transient(&tracks);
        Ok(tracks
            .iter()
            .map(|track| crate::wire::track_info(track, &config))
            .collect())
    }

    pub(super) fn pin_radio_seed(
        key: &str,
        fallback: Option<reader::Track>,
        tracks: Vec<reader::Track>,
    ) -> Vec<reader::Track> {
        if tracks.is_empty() {
            return tracks;
        }
        let (seed_rows, mut rest): (Vec<_>, Vec<_>) = tracks
            .into_iter()
            .partition(|track| track.id.key().as_ref() == key);
        if let Some(seed) = seed_rows.into_iter().next().or(fallback) {
            rest.insert(0, seed);
        }
        rest
    }

    pub async fn playlist_radio(&self, id: &str) -> Result<Vec<api::TrackInfo>, ApiError> {
        if id.trim().is_empty() {
            return Err(ApiError::invalid_input("playlist radio seed is required"));
        }
        let config = self.current().await;
        let tracks = self
            .source()
            .await
            .start_playlist_radio(id)
            .await
            .map_err(Self::source_error)?;
        self.library.register_transient(&tracks);
        Ok(tracks
            .iter()
            .map(|track| crate::wire::track_info(track, &config))
            .collect())
    }

    pub async fn search_radio(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<Vec<api::RadioStationInfo>, ApiError> {
        let stations = if query.trim().is_empty() {
            radio::browser::top_stations(limit).await
        } else {
            radio::browser::search(query, limit).await
        }
        .map_err(|error| ApiError::new(ErrorCode::SourceUnreachable, error.to_string()))?;
        let mut registry = self.registry.write().await;
        let mut result = Vec::with_capacity(stations.len());
        for station in stations {
            let manifest = radio::browser::to_manifest(&station);
            let pinned = registry.is_registry_station(&manifest.id);
            result.push(Self::station_info(&manifest, pinned));
            registry.insert_manifest(manifest);
        }
        let snapshot = Arc::new(registry.clone());
        drop(registry);
        self.library.set_station_registry(snapshot);
        Ok(result)
    }

    pub async fn radio_registries(&self) -> Vec<api::RadioRegistryInfo> {
        self.current()
            .await
            .radio_registries
            .iter()
            .map(|entry| api::RadioRegistryInfo {
                url: entry.url.clone(),
                enabled: entry.enabled,
                built_in: entry.is_default,
            })
            .collect()
    }

    async fn mutate_radio_config(
        &self,
        key: &'static str,
        mutate: impl FnOnce(&mut config::AppConfig),
    ) -> Result<(), ApiError> {
        self.config.ensure_unlocked(&[key])?;
        let updated = self.config.mutate_state(mutate).await?;
        self.session.set_config(updated, vec![key.to_string()]);
        self.reload_radio().await
    }

    pub async fn add_radio_registry(&self, url: &str) -> Result<(), ApiError> {
        let mut probe = radio::registry::StationRegistry::new();
        probe
            .import_registry(url)
            .await
            .map_err(|error| ApiError::invalid_input(error.to_string()))?;
        let url = url.to_string();
        self.mutate_radio_config("radio_registries", move |config| {
            if !config.radio_registries.iter().any(|entry| entry.url == url) {
                config.radio_registries.push(config::RegistryEntry {
                    url,
                    enabled: true,
                    is_default: false,
                });
            }
        })
        .await
    }

    pub async fn remove_radio_registry(&self, url: &str) -> Result<(), ApiError> {
        let url = url.to_string();
        self.mutate_radio_config("radio_registries", move |config| {
            config
                .radio_registries
                .retain(|entry| entry.url != url || entry.is_default);
        })
        .await
    }

    pub async fn set_radio_registry_enabled(
        &self,
        url: &str,
        enabled: bool,
    ) -> Result<(), ApiError> {
        let url = url.to_string();
        self.mutate_radio_config("radio_registries", move |config| {
            if let Some(entry) = config
                .radio_registries
                .iter_mut()
                .find(|entry| entry.url == url)
            {
                entry.enabled = enabled;
            }
        })
        .await
    }

    pub async fn pin_station(
        &self,
        station: api::RadioStationInfo,
        pinned: bool,
    ) -> Result<(), ApiError> {
        let manifest = {
            let registry = self.registry.read().await;
            registry
                .get(&station.id)
                .cloned()
                .unwrap_or_else(|| radio::manifest::StationManifest {
                    schema_version: "1.0".to_string(),
                    id: station.id.clone(),
                    name: station.name.clone(),
                    description: station.description.clone(),
                    icon: station.icon.clone(),
                    tags: station.tags.clone(),
                    streams: station
                        .streams
                        .iter()
                        .map(|stream| radio::manifest::StreamDef {
                            id: stream.id.clone(),
                            name: stream.name.clone(),
                            url: stream.url.clone(),
                            codec: None,
                            bitrate: None,
                            icon: stream.icon.clone(),
                        })
                        .collect(),
                    metadata: Some(radio::manifest::MetadataSourceDef::Static(
                        radio::manifest::StaticSourceDef {
                            title: station.name.clone(),
                            artist: "Live Radio".to_string(),
                            cover_url: station.artwork.clone(),
                            stream_overrides: std::collections::HashMap::new(),
                        },
                    )),
                })
        };
        manifest
            .validate()
            .map_err(|error| ApiError::invalid_input(error.to_string()))?;
        let id = manifest.id.clone();
        let json = serde_json::to_string(&manifest)
            .map_err(|error| ApiError::internal(error.to_string()))?;
        self.mutate_radio_config("pinned_stations", move |config| {
            config.pinned_stations.retain(|existing| {
                serde_json::from_str::<radio::manifest::StationManifest>(existing)
                    .map(|station| station.id != id)
                    .unwrap_or(true)
            });
            if pinned {
                config.pinned_stations.push(json);
            }
        })
        .await
    }
}
