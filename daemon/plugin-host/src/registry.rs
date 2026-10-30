//! Every loaded plugin, and which one serves each service.

use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::manifest::Manifest;
use crate::plugin::{Code, Origin, Plugin};

pub const MANIFEST_FILE: &str = "kopuz-plugin.toml";
pub const COMPONENT_FILE: &str = "plugin.wasm";

/// A plugin shipped inside kopuz: a component, or none for a native plugin.
#[derive(Debug, Clone, Copy)]
pub struct Bundled {
    pub manifest: &'static str,
    pub wasm: Option<&'static [u8]>,
}

#[derive(Debug, Default)]
pub struct Registry {
    plugins: Vec<Arc<Plugin>>,
    by_service: HashMap<String, Arc<Plugin>>,
}

impl Registry {
    /// The bundled plugins, then each plugin installed under `dir`. An
    /// installed plugin with a bundled plugin's id replaces it, which is how a
    /// fix or a fork of an official plugin is tried without rebuilding kopuz.
    pub fn load(bundled: &[Bundled], dir: Option<&Path>) -> Self {
        let mut registry = Self::default();
        for plugin in bundled {
            let code = match plugin.wasm {
                Some(wasm) => Code::Wasm(Cow::Borrowed(wasm)),
                None => Code::Native,
            };
            match Manifest::parse(plugin.manifest) {
                Ok(manifest) => registry.add(Plugin::new(manifest, code, Origin::Bundled)),
                Err(error) => tracing::error!(%error, "a bundled plugin's manifest is invalid"),
            }
        }
        if let Some(dir) = dir {
            for plugin in installed(dir) {
                registry.add(plugin);
            }
        }
        registry
    }

    fn add(&mut self, plugin: Plugin) {
        let plugin = Arc::new(plugin);
        if let Some(replaced) = self
            .plugins
            .iter()
            .position(|loaded| loaded.id() == plugin.id())
        {
            let old = self.plugins.remove(replaced);
            self.by_service
                .retain(|_, serving| !Arc::ptr_eq(serving, &old));
            tracing::info!(
                plugin = %plugin.id(),
                version = %plugin.manifest().version,
                "an installed plugin replaces the bundled one"
            );
        }
        for service in &plugin.manifest().services {
            if let Some(other) = self.by_service.get(&service.id) {
                tracing::warn!(
                    service = %service.id,
                    serving = %other.id(),
                    ignored = %plugin.id(),
                    "two plugins serve one service; keeping the first"
                );
                continue;
            }
            self.by_service
                .insert(service.id.clone(), Arc::clone(&plugin));
        }
        self.plugins.push(plugin);
    }

    /// The plugin serving `service`, if any does.
    pub fn for_service(&self, service: &str) -> Option<&Arc<Plugin>> {
        self.by_service.get(service)
    }

    pub fn plugins(&self) -> &[Arc<Plugin>] {
        &self.plugins
    }
}

/// Each `<dir>/<name>/` holding a manifest and a component. One that fails to
/// load is logged and skipped, so a broken plugin never stops the rest.
fn installed(dir: &Path) -> Vec<Plugin> {
    plugin_dirs(dir)
        .into_iter()
        .filter_map(|path| match load_dir(&path) {
            Ok(plugin) => Some(plugin),
            Err(error) => {
                tracing::warn!(plugin = %path.display(), %error, "plugin not loaded");
                None
            }
        })
        .collect()
}

/// A host allowlist as a set, so two lists agree whatever their order or case.
pub fn host_set(network: &[String]) -> BTreeSet<String> {
    network
        .iter()
        .map(|host| host.trim().to_ascii_lowercase())
        .collect()
}

/// Each `<dir>/<name>/`, sorted, so which of two plugins wins a service does
/// not depend on directory order. A dot directory is an install or removal in
/// progress, or one a crash left behind.
pub fn plugin_dirs(dir: &Path) -> Vec<PathBuf> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            tracing::warn!(dir = %dir.display(), %error, "plugin directory unreadable");
            return Vec::new();
        }
    };
    let mut paths: Vec<_> = entries
        .filter_map(Result::ok)
        .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    paths.sort();
    paths
}

/// Every directory under `dir` whose manifest names `id`, loaded or not. The
/// directory name need not be the id.
pub fn dirs_of(dir: &Path, id: &str) -> Vec<PathBuf> {
    plugin_dirs(dir)
        .into_iter()
        .filter(|path| {
            std::fs::read_to_string(path.join(MANIFEST_FILE))
                .ok()
                .and_then(|text| Manifest::parse(&text).ok())
                .is_some_and(|manifest| manifest.id == id)
        })
        .collect()
}

fn load_dir(dir: &Path) -> Result<Plugin, String> {
    let manifest = std::fs::read_to_string(dir.join(MANIFEST_FILE))
        .map_err(|error| format!("{MANIFEST_FILE}: {error}"))?;
    let manifest = Manifest::parse(&manifest).map_err(|error| error.to_string())?;
    let wasm = std::fs::read(dir.join(COMPONENT_FILE))
        .map_err(|error| format!("{COMPONENT_FILE}: {error}"))?;
    let plugin = Plugin::new(manifest, Code::Wasm(Cow::Owned(wasm)), Origin::Installed);
    // Checked here, or a broken copy of a bundled plugin would take its
    // services away and fail only on the first call.
    plugin.compile().map_err(|error| error.to_string())?;
    Ok(plugin)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The smallest valid component: the header and nothing else.
    const EMPTY_COMPONENT: &[u8] = b"\0asm\x0d\x00\x01\x00";

    fn manifest(id: &str, version: &str, services: &[&str]) -> String {
        let mut text =
            format!("id = \"{id}\"\nname = \"{id}\"\nversion = \"{version}\"\napi = \"0.1\"\n");
        for service in services {
            text.push_str(&format!("[[service]]\nid = \"{service}\"\n"));
        }
        text
    }

    #[test]
    fn an_installed_plugin_replaces_the_bundled_one_with_its_id() {
        let bundled = manifest("subsonic", "0.1.0", &["subsonic", "custom"]);
        let bundled = [Bundled {
            manifest: Box::leak(bundled.into_boxed_str()),
            wasm: Some(b""),
        }];
        let dir = std::env::temp_dir().join(format!("kopuz-plugins-{}", std::process::id()));
        let fork = dir.join("subsonic-fork");
        std::fs::create_dir_all(&fork).unwrap();
        std::fs::write(
            fork.join(MANIFEST_FILE),
            manifest("subsonic", "0.2.0", &["subsonic"]),
        )
        .unwrap();
        std::fs::write(fork.join(COMPONENT_FILE), EMPTY_COMPONENT).unwrap();
        let broken = dir.join("broken");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join(MANIFEST_FILE), "id = ").unwrap();

        let registry = Registry::load(&bundled, Some(&dir));
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(registry.plugins().len(), 1);
        let serving = registry.for_service("subsonic").expect("subsonic served");
        assert_eq!(serving.manifest().version, "0.2.0");
        assert_eq!(serving.origin(), Origin::Installed);
        assert!(
            registry.for_service("custom").is_none(),
            "the fork dropped the custom service, so nothing serves it"
        );
    }

    #[test]
    fn the_first_plugin_to_claim_a_service_keeps_it() {
        let first = manifest("a", "1.0.0", &["jellyfin"]);
        let second = manifest("b", "1.0.0", &["jellyfin"]);
        let bundled = [
            Bundled {
                manifest: Box::leak(first.into_boxed_str()),
                wasm: Some(b""),
            },
            Bundled {
                manifest: Box::leak(second.into_boxed_str()),
                wasm: Some(b""),
            },
        ];
        let registry = Registry::load(&bundled, None);
        assert_eq!(registry.plugins().len(), 2);
        assert_eq!(registry.for_service("jellyfin").map(|p| p.id()), Some("a"));
    }

    #[test]
    fn a_native_plugin_has_no_component_and_a_component_can_replace_it() {
        let native = [Bundled {
            manifest: Box::leak(manifest("ytmusic", "1.0.0", &["ytmusic"]).into_boxed_str()),
            wasm: None,
        }];
        let registry = Registry::load(&native, None);
        assert!(registry.for_service("ytmusic").unwrap().is_native());

        let dir = std::env::temp_dir().join(format!("kopuz-plugins-native-{}", std::process::id()));
        let port = dir.join("ytmusic");
        std::fs::create_dir_all(&port).unwrap();
        std::fs::write(
            port.join(MANIFEST_FILE),
            manifest("ytmusic", "2.0.0", &["ytmusic"]),
        )
        .unwrap();
        std::fs::write(port.join(COMPONENT_FILE), EMPTY_COMPONENT).unwrap();
        let registry = Registry::load(&native, Some(&dir));
        std::fs::remove_dir_all(&dir).ok();
        assert!(!registry.for_service("ytmusic").unwrap().is_native());
    }

    #[test]
    fn a_staging_directory_is_not_a_second_copy() {
        let dir =
            std::env::temp_dir().join(format!("kopuz-plugins-staging-{}", std::process::id()));
        for name in ["radio", ".radio.installing", ".radio.removing"] {
            let path = dir.join(name);
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(
                path.join(MANIFEST_FILE),
                manifest("radio", "1.0.0", &["radio"]),
            )
            .unwrap();
            std::fs::write(path.join(COMPONENT_FILE), EMPTY_COMPONENT).unwrap();
        }
        let registry = Registry::load(&[], Some(&dir));
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(registry.plugins().len(), 1);
    }

    #[test]
    fn a_copy_that_does_not_compile_leaves_the_bundled_plugin() {
        let bundled = [Bundled {
            manifest: Box::leak(manifest("subsonic", "0.1.0", &["subsonic"]).into_boxed_str()),
            wasm: Some(EMPTY_COMPONENT),
        }];
        let dir = std::env::temp_dir().join(format!("kopuz-plugins-broken-{}", std::process::id()));
        let fork = dir.join("subsonic");
        std::fs::create_dir_all(&fork).unwrap();
        std::fs::write(
            fork.join(MANIFEST_FILE),
            manifest("subsonic", "0.2.0", &["subsonic"]),
        )
        .unwrap();
        std::fs::write(fork.join(COMPONENT_FILE), b"not a component").unwrap();
        let registry = Registry::load(&bundled, Some(&dir));
        std::fs::remove_dir_all(&dir).ok();
        let serving = registry.for_service("subsonic").expect("subsonic served");
        assert_eq!(serving.origin(), Origin::Bundled);
    }
}
