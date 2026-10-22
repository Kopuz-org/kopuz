//! The plugins the daemon runs, and the community marketplace it installs
//! more from.

use std::sync::Arc;
use std::time::{Duration, Instant};

use api::{ApiError, PluginCatalog, PluginInfo, PluginOrigin};
use plugin_host::market::{self, Index, Listing, MarketError};
use tokio::sync::Mutex;

/// `KOPUZ_PLUGIN_INDEX` overrides it; a `file://` index works for testing.
pub const DEFAULT_INDEX_URL: &str =
    "https://raw.githubusercontent.com/Kopuz-org/plugins/main/index.json";

const INDEX_TTL: Duration = Duration::from_secs(10 * 60);

const MAX_INDEX_BYTES: usize = 4 * 1024 * 1024;

pub struct PluginService {
    index_url: String,
    http: reqwest::Client,
    index: Mutex<Option<(Instant, Arc<Index>)>>,
}

impl Default for PluginService {
    fn default() -> Self {
        Self::new(
            std::env::var("KOPUZ_PLUGIN_INDEX").unwrap_or_else(|_| DEFAULT_INDEX_URL.to_string()),
        )
    }
}

fn market_error(error: MarketError) -> ApiError {
    match error {
        MarketError::NotListed(_) => ApiError::not_found(error.to_string()),
        MarketError::Io(_) => ApiError::internal(error.to_string()),
        _ => ApiError::invalid_input(error.to_string()),
    }
}

impl PluginService {
    pub fn new(index_url: String) -> Self {
        let http = utils::http::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap_or_else(|_| utils::http::client());
        Self {
            index_url,
            http,
            index: Mutex::new(None),
        }
    }

    async fn fetch(&self, url: &str, limit: usize) -> Result<Vec<u8>, String> {
        if let Some(path) = url.strip_prefix("file://") {
            let bytes = tokio::fs::read(path)
                .await
                .map_err(|error| format!("{path}: {error}"))?;
            return match bytes.len() <= limit {
                true => Ok(bytes),
                false => Err(format!("{url} is too large")),
            };
        }
        let parsed = reqwest::Url::parse(url).map_err(|error| format!("{url}: {error}"))?;
        // A plugin is code: never fetch it over a connection open to tampering.
        if parsed.scheme() != "https" {
            return Err(format!("{url} is not an https address"));
        }
        let mut response = self
            .http
            .get(parsed)
            .send()
            .await
            .map_err(|error| error.without_url().to_string())?;
        if !response.status().is_success() {
            return Err(format!("{url} answered {}", response.status()));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| error.without_url().to_string())?
        {
            if body.len() + chunk.len() > limit {
                return Err(format!("{url} is too large"));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }

    async fn index(&self, refresh: bool) -> Result<Arc<Index>, String> {
        let mut cached = self.index.lock().await;
        if !refresh
            && let Some((fetched, index)) = cached.as_ref()
            && fetched.elapsed() < INDEX_TTL
        {
            return Ok(Arc::clone(index));
        }
        let bytes = self.fetch(&self.index_url, MAX_INDEX_BYTES).await?;
        let index = Arc::new(Index::parse(&bytes).map_err(|error| error.to_string())?);
        *cached = Some((Instant::now(), Arc::clone(&index)));
        Ok(index)
    }

    pub async fn catalog(&self) -> PluginCatalog {
        let mut plugins = installed();
        let marketplace_error = match self.index(false).await {
            Ok(index) => {
                let official = server::plugins::official_ids();
                for listing in &index.plugins {
                    if official.contains(&listing.id) {
                        continue;
                    }
                    match plugins.iter_mut().find(|info| info.id == listing.id) {
                        Some(info) => {
                            info.available_version = Some(listing.version.clone());
                            if info.homepage.is_none() {
                                info.homepage = listing.homepage.clone();
                            }
                            if info.author.is_empty() {
                                info.author = listing.author.clone();
                            }
                        }
                        None => plugins.push(listed(listing)),
                    }
                }
                None
            }
            Err(error) => {
                tracing::warn!(%error, url = %self.index_url, "plugin marketplace unreadable");
                Some(error)
            }
        };
        PluginCatalog {
            plugins,
            marketplace_error,
        }
    }

    /// Fetch, verify, check the manifest and compile it once before anything
    /// is written.
    pub async fn install(&self, id: &str) -> Result<PluginInfo, ApiError> {
        let dir = server::plugins::dir()
            .ok_or_else(|| ApiError::unsupported("this daemon has no plugin directory"))?
            .to_path_buf();
        let index = self
            .index(true)
            .await
            .map_err(|error| ApiError::new(api::ErrorCode::SourceUnreachable, error))?;
        let listing = index.listing(id).map_err(market_error)?;
        market::check_listing(listing, &server::plugins::official_ids()).map_err(market_error)?;

        let fetch_error = |error: String| ApiError::new(api::ErrorCode::SourceUnreachable, error);
        let manifest_bytes = self
            .fetch(&listing.manifest.url, market::MAX_MANIFEST_BYTES)
            .await
            .map_err(fetch_error)?;
        market::verify(&listing.manifest, &manifest_bytes).map_err(market_error)?;
        let component = self
            .fetch(&listing.component.url, market::MAX_COMPONENT_BYTES)
            .await
            .map_err(fetch_error)?;
        market::verify(&listing.component, &component).map_err(market_error)?;

        let manifest_text = String::from_utf8(manifest_bytes)
            .map_err(|_| ApiError::invalid_input("the plugin's manifest is not text"))?;
        let built_in: Vec<&str> = config::MusicService::ALL
            .iter()
            .map(|service| service.id())
            .chain([config::MusicService::Plugin.id(), crate::services::FOLDERS])
            .collect();
        let manifest =
            market::check_manifest(listing, &manifest_text, &built_in).map_err(market_error)?;

        let candidate = plugin_host::Plugin::new(
            manifest,
            plugin_host::Code::Wasm(std::borrow::Cow::Owned(component.clone())),
            plugin_host::Origin::Installed,
        );
        candidate
            .prepare()
            .await
            .map_err(|error| ApiError::invalid_input(error.to_string()))?;

        let id = listing.id.clone();
        tokio::task::spawn_blocking(move || market::install(&dir, &id, &manifest_text, &component))
            .await
            .map_err(|error| ApiError::internal(error.to_string()))?
            .map_err(market_error)?;
        server::plugins::reload();
        tracing::info!(plugin = %listing.id, version = %listing.version, "plugin installed");

        let mut info = installed()
            .into_iter()
            .find(|info| info.id == listing.id)
            .ok_or_else(|| ApiError::internal("the installed plugin did not load"))?;
        info.available_version = Some(listing.version.clone());
        Ok(info)
    }

    pub async fn uninstall(&self, id: &str) -> Result<(), ApiError> {
        let registry = server::plugins::registry();
        let plugin = registry
            .plugins()
            .iter()
            .find(|plugin| plugin.id() == id)
            .ok_or_else(|| ApiError::not_found("no such plugin is installed"))?;
        if plugin.origin() == plugin_host::Origin::Bundled {
            return Err(ApiError::invalid_input(
                "an official plugin ships with kopuz and cannot be removed",
            ));
        }
        let dir = server::plugins::dir()
            .ok_or_else(|| ApiError::unsupported("this daemon has no plugin directory"))?
            .to_path_buf();
        let id = id.to_string();
        let removed = id.clone();
        tokio::task::spawn_blocking(move || market::uninstall(&dir, &removed))
            .await
            .map_err(|error| ApiError::internal(error.to_string()))?
            .map_err(market_error)?;
        server::plugins::reload();
        tracing::info!(plugin = %id, "plugin removed");
        Ok(())
    }
}

fn installed() -> Vec<PluginInfo> {
    let registry = server::plugins::registry();
    registry
        .plugins()
        .iter()
        .map(|plugin| {
            let manifest = plugin.manifest();
            PluginInfo {
                id: manifest.id.clone(),
                name: manifest.name.clone(),
                description: manifest.description.clone(),
                author: String::new(),
                homepage: None,
                origin: match plugin.origin() {
                    plugin_host::Origin::Bundled => PluginOrigin::Official,
                    plugin_host::Origin::Installed => PluginOrigin::Community,
                },
                sandboxed: !plugin.is_native(),
                installed_version: Some(manifest.version.clone()),
                available_version: None,
                services: manifest
                    .services
                    .iter()
                    .map(|service| service.id.clone())
                    .collect(),
                network: manifest.permissions.network.clone(),
            }
        })
        .collect()
}

fn listed(listing: &Listing) -> PluginInfo {
    PluginInfo {
        id: listing.id.clone(),
        name: listing.name.clone(),
        description: listing.description.clone(),
        author: listing.author.clone(),
        homepage: listing.homepage.clone(),
        origin: PluginOrigin::Community,
        sandboxed: true,
        installed_version: None,
        available_version: Some(listing.version.clone()),
        services: listing.services.clone(),
        network: listing.network.clone(),
    }
}
