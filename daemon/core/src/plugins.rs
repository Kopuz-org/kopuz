//! The plugins the daemon runs, and the community marketplace it installs
//! more from.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant, SystemTime};

use api::{ApiError, PluginCatalog, PluginInfo, PluginOrigin, PluginUpdate};
use plugin_host::market::{self, Index, Listing, MarketError};
use tokio::sync::Mutex;

/// `KOPUZ_PLUGIN_INDEX` overrides it; a `file://` index works for testing.
pub const DEFAULT_INDEX_URL: &str =
    "https://raw.githubusercontent.com/Kopuz-org/plugins/main/index.json";

const INDEX_TTL: Duration = Duration::from_secs(10 * 60);

const MAX_INDEX_BYTES: usize = 4 * 1024 * 1024;

/// How often the plugin directory is checked for files dropped in by hand.
const WATCH_INTERVAL: Duration = Duration::from_secs(2);

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
        let registry = server::plugins::registry();
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
                            let loaded = registry
                                .plugins()
                                .iter()
                                .find(|plugin| plugin.id() == listing.id);
                            // A republished build under the same version is an update too.
                            if loaded.is_some_and(|plugin| differs(plugin, listing)) {
                                info.update = Some(PluginUpdate {
                                    version: listing.version.clone(),
                                    network: listing.network.clone(),
                                    added_hosts: added(&info.network, &listing.network),
                                });
                            }
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
        // Files waiting in the plugin directory win over the marketplace: they
        // are what an approval would load.
        // One that waited across a restart has nothing loaded to attach to.
        for waiting in registry.pending() {
            let manifest = waiting.plugin.manifest();
            let update = PluginUpdate {
                version: manifest.version.clone(),
                network: manifest.permissions.network.clone(),
                added_hosts: waiting.added_hosts.clone(),
            };
            match plugins.iter_mut().find(|info| info.id == manifest.id) {
                Some(info) => info.update = Some(update),
                None => plugins.push(PluginInfo {
                    id: manifest.id.clone(),
                    name: manifest.name.clone(),
                    description: manifest.description.clone(),
                    author: String::new(),
                    homepage: None,
                    origin: PluginOrigin::Community,
                    sandboxed: true,
                    installed_version: None,
                    available_version: Some(manifest.version.clone()),
                    services: manifest
                        .services
                        .iter()
                        .map(|service| service.id.clone())
                        .collect(),
                    network: Vec::new(),
                    update: Some(update),
                }),
            }
        }
        PluginCatalog {
            plugins,
            marketplace_error,
        }
    }

    /// Install or update `id`, swapping it in while sources keep running.
    /// `network` is what the user agreed to; anything else is refused.
    pub async fn install(&self, id: &str, network: &[String]) -> Result<PluginInfo, ApiError> {
        if server::plugins::registry()
            .pending()
            .iter()
            .any(|waiting| waiting.plugin.id() == id)
        {
            return match server::plugins::approve(id, network).await {
                Some(()) => {
                    tracing::info!(plugin = %id, "plugin update allowed its new hosts");
                    installed()
                        .into_iter()
                        .find(|info| info.id == id)
                        .ok_or_else(|| ApiError::internal("the updated plugin did not load"))
                }
                None => Err(changed_hosts()),
            };
        }
        self.install_listed(id, network).await
    }

    /// Fetch, verify, check the manifest and compile it once before anything
    /// is written.
    async fn install_listed(&self, id: &str, network: &[String]) -> Result<PluginInfo, ApiError> {
        let dir = server::plugins::dir()
            .ok_or_else(|| ApiError::unsupported("this daemon has no plugin directory"))?
            .to_path_buf();
        let index = self
            .index(true)
            .await
            .map_err(|error| ApiError::new(api::ErrorCode::SourceUnreachable, error))?;
        let listing = index.listing(id).map_err(market_error)?;
        market::check_listing(listing, &server::plugins::official_ids()).map_err(market_error)?;
        // The index is fetched again here, so what it lists now may not be
        // what the user was shown.
        if plugin_host::host_set(&listing.network) != plugin_host::host_set(network) {
            return Err(changed_hosts());
        }

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
        // A hand-installed copy of the id under another directory name would
        // otherwise stay on disk and win a later rescan.
        let copies = server::plugins::registry().dirs_of(&id);
        server::plugins::adopt(Arc::new(candidate), move || {
            let target = market::install(&dir, &id, &manifest_text, &component)?;
            for copy in copies.iter().filter(|copy| **copy != target) {
                market::uninstall(&dir, copy)?;
            }
            Ok(target)
        })
        .await
        .map_err(market_error)?;
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
        if registry
            .plugins()
            .iter()
            .any(|plugin| plugin.id() == id && plugin.origin() == plugin_host::Origin::Bundled)
        {
            return Err(ApiError::invalid_input(
                "an official plugin ships with kopuz and cannot be removed",
            ));
        }
        // An update that waited across a restart has files but nothing loaded.
        let copies = registry.dirs_of(id);
        if copies.is_empty() {
            return Err(ApiError::not_found("no such plugin is installed"));
        }
        let dir = server::plugins::dir()
            .ok_or_else(|| ApiError::unsupported("this daemon has no plugin directory"))?
            .to_path_buf();
        server::plugins::remove(move || {
            copies
                .iter()
                .try_for_each(|copy| market::uninstall(&dir, copy))
        })
        .await
        .map_err(market_error)?;
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
                update: None,
            }
        })
        .collect()
}

fn added(installed: &[String], update: &[String]) -> Vec<String> {
    let allowed = plugin_host::host_set(installed);
    plugin_host::host_set(update)
        .into_iter()
        .filter(|host| !allowed.contains(host))
        .collect()
}

fn changed_hosts() -> ApiError {
    ApiError::invalid_input(
        "the plugin now reaches different hosts than the ones shown; review them and try again",
    )
}

/// Size and modification time of each plugin's files, to notice a change
/// without reading or hashing them.
type Stamps = Vec<(PathBuf, Option<(u64, SystemTime)>)>;

fn stamps(dir: &Path) -> Stamps {
    plugin_host::plugin_dirs(dir)
        .into_iter()
        .flat_map(|plugin| {
            [plugin_host::MANIFEST_FILE, plugin_host::COMPONENT_FILE].map(|file| plugin.join(file))
        })
        .map(|path| {
            let stamp = std::fs::metadata(&path)
                .ok()
                .and_then(|meta| Some((meta.len(), meta.modified().ok()?)));
            (path, stamp)
        })
        .collect()
}

/// Watch the plugin directory and swap in plugins whose files changed,
/// calling `changed` when that loaded or queued anything. Stops once `owner`
/// is dropped.
pub fn watch<T, F, Fut>(owner: Weak<T>, changed: F)
where
    T: Send + Sync + 'static,
    F: Fn(Arc<T>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send,
{
    let Some(dir) = server::plugins::dir() else {
        return;
    };
    tokio::spawn(async move {
        // Empty, so the first quiet poll rescans and an edit made between boot
        // loading the plugins and this task starting is not missed.
        let mut loaded = Stamps::new();
        let mut last = stamps(dir);
        loop {
            tokio::time::sleep(WATCH_INTERVAL).await;
            if owner.strong_count() == 0 {
                return;
            }
            let Ok(now) = tokio::task::spawn_blocking(move || stamps(dir)).await else {
                continue;
            };
            // Wait for one quiet interval, so files still being copied in are
            // not read half written.
            let settled = now == last;
            last.clone_from(&now);
            if !settled || now == loaded {
                continue;
            }
            loaded = now;
            if server::plugins::rescan().await {
                let Some(owner) = owner.upgrade() else {
                    return;
                };
                changed(owner).await;
            }
        }
    });
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
        update: None,
    }
}

/// Versions are free-form, so any other version or component counts; nothing
/// says which of two is newer.
fn differs(plugin: &plugin_host::Plugin, listing: &Listing) -> bool {
    plugin.manifest().version != listing.version
        || plugin
            .digest()
            .is_some_and(|digest| !digest.eq_ignore_ascii_case(listing.component.sha256.trim()))
}
