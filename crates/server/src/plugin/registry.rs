//! The process-wide set of plugins: what is installed, and what this run has
//! loaded.
//!
//! Two pieces of state, deliberately kept apart. The manifest list answers "what
//! could the user pick" and is refreshed by [`PluginRegistry::rescan`]; the
//! instance map answers "what is running" and only ever grows on demand. A
//! rescan therefore never disturbs a loaded plugin, and listing plugins never
//! loads one.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use super::manifest::{self, PluginManifest};
use super::{Handshake, PluginError, PluginInstance};
use crate::source::Capabilities;

static REGISTRY: OnceLock<PluginRegistry> = OnceLock::new();

/// The registry. The first call scans the filesystem, every later one is free.
pub fn registry() -> &'static PluginRegistry {
    REGISTRY.get_or_init(PluginRegistry::new)
}

/// Unload every plugin this run loaded. A no-op when none was, so app shutdown
/// never triggers a filesystem scan just to tear down nothing.
pub async fn shutdown_all() {
    let Some(existing) = REGISTRY.get() else {
        return;
    };
    existing.shutdown_all().await;
}

/// The part of a handshake a synchronous caller still needs after the call that
/// produced it has returned.
struct HandshakeFacts {
    capabilities: Capabilities,
    web_url_template: Option<String>,
}

pub struct PluginRegistry {
    manifests: RwLock<Vec<PluginManifest>>,
    facts: RwLock<HashMap<String, HandshakeFacts>>,
    instances: tokio::sync::Mutex<HashMap<String, Arc<PluginInstance>>>,
}

impl PluginRegistry {
    fn new() -> Self {
        Self {
            manifests: RwLock::new(manifest::discover()),
            facts: RwLock::new(HashMap::new()),
            instances: tokio::sync::Mutex::new(HashMap::new()),
        }
    }

    /// Cached manifests from the last scan.
    pub fn manifests(&self) -> Vec<PluginManifest> {
        match self.manifests.read() {
            Ok(manifests) => manifests.to_vec(),
            Err(_) => Vec::new(),
        }
    }

    pub fn manifest(&self, id: &str) -> Option<PluginManifest> {
        let manifests = self.manifests.read().ok()?;
        manifests.iter().find(|m| m.id == id).cloned()
    }

    /// Re-scan the filesystem. Does not touch loaded plugins: a rescan is about
    /// what is installed, not what is running.
    pub fn rescan(&self) {
        // Scanned before taking the lock so a slow directory never blocks a
        // render path reading the list.
        let found = manifest::discover();
        if let Ok(mut manifests) = self.manifests.write() {
            *manifests = found;
        }
    }

    /// The loaded instance, loading it on first use.
    ///
    /// The instance map is held across the whole load, so two racing callers get
    /// the same instance rather than two states over one data directory. A load
    /// that fails is not remembered: the next call tries again, since building a
    /// Lua state is cheap and there is no process to have crashed.
    pub async fn instance(&self, id: &str) -> Result<Arc<PluginInstance>, PluginError> {
        let mut instances = self.instances.lock().await;
        if let Some(loaded) = instances.get(id) {
            return Ok(Arc::clone(loaded));
        }
        let manifest = self
            .manifest(id)
            .ok_or_else(|| PluginError::NotInstalled(id.to_string()))?;
        let instance = PluginInstance::load(manifest).await?;
        self.remember(id, instance.handshake());
        instances.insert(id.to_string(), Arc::clone(&instance));
        Ok(instance)
    }

    /// Capabilities from the last handshake, for synchronous render paths that
    /// cannot await a load.
    pub fn cached_capabilities(&self, id: &str) -> Option<Capabilities> {
        let facts = self.facts.read().ok()?;
        facts.get(id).map(|plugin| plugin.capabilities)
    }

    pub fn cached_web_url_template(&self, id: &str) -> Option<String> {
        let facts = self.facts.read().ok()?;
        facts.get(id)?.web_url_template.clone()
    }

    /// Whether the plugin is loaded, without loading it. Answered from the facts
    /// map rather than the instance map: the facts are written under the same
    /// mutex the loader holds, and reading them needs no `await`, which a render
    /// path does not have.
    pub fn loaded(&self, id: &str) -> bool {
        self.facts.read().is_ok_and(|facts| facts.contains_key(id))
    }

    /// Drop one plugin's state, running its `unload` first. A caller still
    /// holding an `Arc` keeps a working instance until it drops it; the next
    /// [`instance`](Self::instance) builds a fresh one either way.
    pub async fn disconnect(&self, id: &str) {
        let dropped = {
            let mut instances = self.instances.lock().await;
            instances.remove(id)
        };
        self.forget(id);
        if let Some(instance) = dropped {
            instance.unload().await;
        }
    }

    /// [`disconnect`](Self::disconnect) for every loaded plugin, in one pass.
    pub async fn shutdown_all(&self) {
        let dropped: Vec<(String, Arc<PluginInstance>)> = {
            let mut instances = self.instances.lock().await;
            instances.drain().collect()
        };
        for (id, instance) in dropped {
            self.forget(&id);
            instance.unload().await;
        }
    }

    fn remember(&self, id: &str, handshake: &Handshake) {
        if let Ok(mut facts) = self.facts.write() {
            facts.insert(
                id.to_string(),
                HandshakeFacts {
                    capabilities: handshake.capabilities,
                    web_url_template: handshake.web_url_template.clone(),
                },
            );
        }
    }

    fn forget(&self, id: &str) {
        if let Ok(mut facts) = self.facts.write() {
            facts.remove(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A registry with nothing installed. Built by hand because
    /// [`PluginRegistry::new`] scans the real config directory.
    fn empty() -> PluginRegistry {
        PluginRegistry {
            manifests: RwLock::new(Vec::new()),
            facts: RwLock::new(HashMap::new()),
            instances: tokio::sync::Mutex::new(HashMap::new()),
        }
    }

    #[tokio::test]
    async fn an_unknown_id_never_loads() {
        let registry = empty();
        assert_eq!(
            registry.instance("nope").await.err(),
            Some(PluginError::NotInstalled("nope".to_string()))
        );

        // Racing callers queue on the instance mutex rather than deadlocking on
        // the manifest lock the loader reads while holding it.
        let (first, second) = tokio::join!(registry.instance("nope"), registry.instance("nope"));
        assert!(first.is_err() && second.is_err());
    }

    #[test]
    fn cached_facts_appear_with_the_load_and_go_with_it() {
        let registry = empty();
        assert!(!registry.loaded("example"));
        assert!(registry.cached_capabilities("example").is_none());

        let handshake = Handshake {
            capabilities: Capabilities {
                radio: true,
                ..Capabilities::default()
            },
            web_url_template: Some("https://example.test/{id}".to_string()),
            ..Handshake::default()
        };
        registry.remember("example", &handshake);

        assert!(registry.loaded("example"));
        assert_eq!(
            registry.cached_capabilities("example").map(|c| c.radio),
            Some(true)
        );
        assert_eq!(
            registry.cached_web_url_template("example").as_deref(),
            Some("https://example.test/{id}")
        );

        registry.forget("example");
        assert!(!registry.loaded("example"));
        assert!(registry.cached_web_url_template("example").is_none());
    }
}
