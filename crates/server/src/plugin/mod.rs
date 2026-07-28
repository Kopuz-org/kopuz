//! Generic plugin support: an external program can provide a music source.
//!
//! Nothing in here knows about any particular service. A plugin is a binary
//! plus a `plugin.toml`, discovered under the config directory; the host spawns
//! it, negotiates [`wire::PROTOCOL_VERSION`], and adapts the result onto
//! [`crate::source::MediaSource`] like any built-in backend. Display name,
//! icon, accent, capabilities, sign-in steps and stream URLs are all runtime
//! data — there is no per-provider code path.
//!
//! * [`wire`] — the protocol as serde types.
//! * [`manifest`] — discovery.
//! * [`client`] — the process supervisor.
//! * `source::plugin` — the `MediaSource` adapter.
//!
//! `docs/plugins.md` documents the protocol for plugin authors.

pub mod client;
pub mod manifest;
pub mod wire;

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

pub use client::{PluginClient, PluginEvent};
pub use manifest::PluginManifest;

use crate::source::{Capabilities, SourceError};

/// The process-wide table of discovered manifests and running plugin children.
///
/// There is exactly one plugin process per installed plugin per app run, so the
/// registry is a genuine singleton rather than something threaded through call
/// sites — [`registry`] is the only way to get one.
#[derive(Clone)]
pub struct PluginRegistry {
    inner: Arc<RegistryInner>,
}

struct RegistryInner {
    /// Cached discovery. Scanning touches the filesystem, and the sidebar reads
    /// it on every render.
    manifests: RwLock<Vec<PluginManifest>>,
    clients: tokio::sync::Mutex<HashMap<String, PluginClient>>,
    /// The last handshake each plugin reported, so the sync render paths — which
    /// cannot await a spawn — still see real capabilities.
    handshakes: RwLock<HashMap<String, HandshakeFacts>>,
}

/// The parts of a handshake that sync call sites need.
#[derive(Clone)]
struct HandshakeFacts {
    capabilities: Capabilities,
    web_url_template: Option<String>,
}

static REGISTRY: OnceLock<PluginRegistry> = OnceLock::new();

/// The process-wide registry, scanning for plugins on first use.
pub fn registry() -> &'static PluginRegistry {
    REGISTRY.get_or_init(PluginRegistry::new)
}

impl PluginRegistry {
    fn new() -> Self {
        let manifests = manifest::discover();
        if !manifests.is_empty() {
            tracing::info!(
                count = manifests.len(),
                ids = ?manifests.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
                "discovered plugins"
            );
        }
        Self {
            inner: Arc::new(RegistryInner {
                manifests: RwLock::new(manifests),
                clients: tokio::sync::Mutex::new(HashMap::new()),
                handshakes: RwLock::new(HashMap::new()),
            }),
        }
    }

    /// Every discovered manifest, from the cache.
    pub fn manifests(&self) -> Vec<PluginManifest> {
        match self.inner.manifests.read() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// The cached manifest with this id.
    pub fn manifest(&self, id: &str) -> Option<PluginManifest> {
        self.manifests().into_iter().find(|m| m.id == id)
    }

    /// Re-scan the plugins directory. Running children are left alone — a
    /// rescan is about what is *installed*, not what is *connected*.
    pub fn rescan(&self) -> Vec<PluginManifest> {
        let found = manifest::discover();
        match self.inner.manifests.write() {
            Ok(mut guard) => *guard = found.clone(),
            Err(poisoned) => *poisoned.into_inner() = found.clone(),
        }
        found
    }

    /// The live client for a plugin, connecting on first use. Deduplicated:
    /// concurrent callers wait on the same spawn instead of racing two children
    /// into existence.
    pub async fn client(&self, plugin_id: &str) -> Result<PluginClient, SourceError> {
        let mut clients = self.inner.clients.lock().await;
        if let Some(existing) = clients.get(plugin_id) {
            if !existing.is_exhausted() {
                return Ok(existing.clone());
            }
            // Past its restart budget: drop it so an explicit reconnect can
            // start from a clean slate.
            clients.remove(plugin_id);
        }

        let manifest = self.manifest(plugin_id).ok_or_else(|| {
            SourceError::Backend(format!("no plugin named {plugin_id} is installed"))
        })?;
        let client = PluginClient::connect(manifest).await?;
        self.remember_handshake(plugin_id, &client);
        clients.insert(plugin_id.to_string(), client.clone());
        Ok(client)
    }

    /// What the plugin last said it supports, or `None` before first contact.
    pub fn cached_capabilities(&self, plugin_id: &str) -> Option<Capabilities> {
        self.facts(plugin_id).map(|f| f.capabilities)
    }

    /// The plugin's last-reported `{id}` web-URL template.
    pub fn cached_web_url_template(&self, plugin_id: &str) -> Option<String> {
        self.facts(plugin_id).and_then(|f| f.web_url_template)
    }

    fn facts(&self, plugin_id: &str) -> Option<HandshakeFacts> {
        match self.inner.handshakes.read() {
            Ok(guard) => guard.get(plugin_id).cloned(),
            Err(poisoned) => poisoned.into_inner().get(plugin_id).cloned(),
        }
    }

    fn remember_handshake(&self, plugin_id: &str, client: &PluginClient) {
        let facts = HandshakeFacts {
            capabilities: client.capabilities(),
            web_url_template: client.web_url("{id}"),
        };
        match self.inner.handshakes.write() {
            Ok(mut guard) => {
                guard.insert(plugin_id.to_string(), facts);
            }
            Err(poisoned) => {
                poisoned.into_inner().insert(plugin_id.to_string(), facts);
            }
        }
    }

    /// The client for a plugin only if one is already connected — for UI that
    /// wants to show state without starting a process.
    pub async fn connected(&self, plugin_id: &str) -> Option<PluginClient> {
        self.inner.clients.lock().await.get(plugin_id).cloned()
    }

    /// Stop and forget one plugin's process.
    pub async fn disconnect(&self, plugin_id: &str) {
        let client = self.inner.clients.lock().await.remove(plugin_id);
        if let Some(client) = client {
            client.shutdown().await;
        }
    }

    /// Stop every running plugin. Called on app shutdown so no child outlives
    /// the app that spawned it.
    pub async fn shutdown_all(&self) {
        let clients: Vec<PluginClient> = self
            .inner
            .clients
            .lock()
            .await
            .drain()
            .map(|(_, c)| c)
            .collect();
        for client in clients {
            client.shutdown().await;
        }
    }
}

/// Stop every plugin process this run started. A no-op when no plugin was ever
/// used, so app shutdown never triggers a filesystem scan just to tear down
/// nothing.
pub async fn shutdown_all() {
    if let Some(registry) = REGISTRY.get() {
        registry.shutdown_all().await;
    }
}

/// Split a namespaced item id (`"<plugin_id>/<item_ref>"`) into its parts.
/// Returns `None` when the id carries no plugin prefix.
pub fn split_item_id(item_id: &str) -> Option<(&str, &str)> {
    let (plugin_id, rest) = item_id.split_once('/')?;
    (manifest::is_valid_id(plugin_id) && !rest.is_empty()).then_some((plugin_id, rest))
}

/// Namespace a plugin-supplied item ref so two plugins can never collide in the
/// database.
pub fn namespace_item_id(plugin_id: &str, item_ref: &str) -> String {
    format!("{plugin_id}/{item_ref}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_ids_round_trip() {
        let id = namespace_item_id("example", "track-1");
        assert_eq!(id, "example/track-1");
        assert_eq!(split_item_id(&id), Some(("example", "track-1")));
    }

    #[test]
    fn unprefixed_ids_are_rejected() {
        assert_eq!(split_item_id("bare"), None);
        assert_eq!(split_item_id("example/"), None);
        assert_eq!(split_item_id("BAD/x"), None);
    }
}
