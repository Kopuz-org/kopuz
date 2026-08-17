//! Plugin discovery: `<config dir>/plugins/<id>/plugin.toml`.
//!
//! The manifest carries only what is needed to *list* a plugin without running
//! it: identity, which Lua file to load, and how to badge it in the UI. What
//! the plugin can actually do comes from `setup()` ([`super::Handshake`]), so a
//! manifest can never claim a capability the script lacks.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Lua API generation this build speaks. Bumped only for a breaking change to
/// the `kopuz` global or to an exported function's signature.
pub const API_VERSION: u32 = 1;

/// Writable directory the host scans for plugins, and where a plugin dropped in
/// by hand belongs.
pub fn plugins_dir() -> PathBuf {
    db::config_dir().join("plugins")
}

/// Every directory scanned for plugins.
///
/// `KOPUZ_PLUGIN_PATH` holds extra roots, separated the way `PATH` is. It exists
/// so a declarative package manager can install plugins read-only and outside
/// the config directory: Nix and Flatpak both hand the app immutable store paths
/// and have nowhere to copy a plugin to.
pub fn plugin_search_paths() -> Vec<PathBuf> {
    let mut roots = vec![plugins_dir()];
    if let Some(extra) = std::env::var_os("KOPUZ_PLUGIN_PATH") {
        roots.extend(std::env::split_paths(&extra).filter(|p| !p.as_os_str().is_empty()));
    }
    roots
}

/// Per-plugin writable state, exposed to the script as `kopuz.data_dir`.
///
/// Deliberately not under the manifest directory: that is read-only whenever the
/// plugin came from a store path, and the script still needs somewhere to keep
/// credentials and caches.
pub fn data_dir_for(id: &str) -> PathBuf {
    db::config_dir().join("plugin-data").join(id)
}

/// The `plugin.toml` body, plus the directory it was found in.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PluginManifest {
    /// Stable identity. Namespaces the plugin's item ids in the database, so it
    /// is restricted to `[a-z0-9_-]+`, because a `:` or `/` here would mis-slice every
    /// track ref downstream.
    pub id: String,
    /// Display name for the sidebar badge and settings row.
    pub name: String,
    pub version: String,
    /// Lua API generation the script targets. Checked against [`API_VERSION`].
    pub api: u32,
    /// The Lua chunk to load, relative to the manifest directory. Must stay
    /// inside it: an absolute path or a `..` escape is rejected, so a manifest
    /// can only ever run code shipped alongside it.
    #[serde(default = "default_entry")]
    pub entry: PathBuf,
    /// Icon class for the source switcher (e.g. `"fa-solid fa-puzzle-piece"`).
    #[serde(default)]
    pub icon: Option<String>,
    /// Brand accent as a CSS colour.
    #[serde(default)]
    pub accent: Option<String>,
    /// Filled in by the loader, never by the TOML.
    #[serde(skip)]
    pub dir: PathBuf,
}

fn default_entry() -> PathBuf {
    PathBuf::from("init.lua")
}

impl PluginManifest {
    /// The entry chunk resolved against the manifest directory.
    pub fn entry_path(&self) -> PathBuf {
        self.dir.join(&self.entry)
    }

    /// This plugin's private state directory. Created by the host, owned by the
    /// plugin, never read or deleted by Kopuz, including when the user removes
    /// the source.
    pub fn data_dir(&self) -> PathBuf {
        data_dir_for(&self.id)
    }

    fn validate(&self) -> Result<(), String> {
        if !is_valid_id(&self.id) {
            return Err(format!(
                "id {:?} must be non-empty and only contain a-z, 0-9, '_' or '-'",
                self.id
            ));
        }
        if self.name.trim().is_empty() {
            return Err("name is empty".to_string());
        }
        if self.api != API_VERSION {
            return Err(format!(
                "api = {} but this build speaks {API_VERSION}",
                self.api
            ));
        }
        if !is_contained_relative(&self.entry) {
            return Err(format!(
                "entry {} must be a relative path inside the plugin directory",
                self.entry.display()
            ));
        }
        let entry = self.entry_path();
        if !entry.is_file() {
            return Err(format!("entry {} does not exist", entry.display()));
        }
        Ok(())
    }
}

/// The id charset. Kept narrow on purpose: ids become the `<plugin_id>/<item>`
/// prefix on every persisted track ref, and the ref parser splits on `:`.
pub fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// Whether `p` stays inside its base once joined: relative, no root, no `..`.
/// Used for `entry` and for every `require` a plugin issues.
pub fn is_contained_relative(p: &Path) -> bool {
    use std::path::Component;
    !p.as_os_str().is_empty() && p.components().all(|c| matches!(c, Component::Normal(_)))
}

/// Every readable manifest across [`plugin_search_paths`]. A bad entry is warned
/// about and skipped: one broken plugin must never hide the rest, and discovery
/// runs on a UI path where an `Err` has nowhere useful to go.
///
/// An id found in more than one root resolves to the first one, so a plugin
/// dropped into the config directory shadows a packaged build of the same id.
pub fn discover() -> Vec<PluginManifest> {
    let mut found: Vec<PluginManifest> = Vec::new();
    for root in plugin_search_paths() {
        for manifest in discover_in(&root) {
            if let Some(shadowed) = found.iter().find(|m| m.id == manifest.id) {
                tracing::debug!(
                    id = %manifest.id,
                    kept = %shadowed.dir.display(),
                    ignored = %manifest.dir.display(),
                    "duplicate plugin id"
                );
                continue;
            }
            found.push(manifest);
        }
    }
    found.sort_by_key(|m| m.name.to_lowercase());
    found
}

/// [`discover`] against an explicit root. Split out so tests do not need the
/// real config directory.
pub fn discover_in(root: &Path) -> Vec<PluginManifest> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            tracing::warn!(dir = %root.display(), error = %e, "cannot read plugins directory");
            return Vec::new();
        }
    };

    let mut found = Vec::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        match load(&dir) {
            Ok(manifest) => found.push(manifest),
            Err(LoadError::Missing) => {}
            Err(LoadError::Invalid(reason)) => {
                tracing::warn!(dir = %dir.display(), reason, "skipping malformed plugin");
            }
        }
    }
    found.sort_by_key(|m| m.name.to_lowercase());
    found
}

/// The discovered manifest with this id, if any.
pub fn find(id: &str) -> Option<PluginManifest> {
    discover().into_iter().find(|m| m.id == id)
}

enum LoadError {
    /// No `plugin.toml` here, so an ordinary non-plugin directory.
    Missing,
    Invalid(String),
}

fn load(dir: &Path) -> Result<PluginManifest, LoadError> {
    let path = dir.join("plugin.toml");
    let body = match std::fs::read_to_string(&path) {
        Ok(body) => body,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(LoadError::Missing),
        Err(e) => return Err(LoadError::Invalid(format!("cannot read plugin.toml: {e}"))),
    };
    let mut manifest: PluginManifest =
        toml::from_str(&body).map_err(|e| LoadError::Invalid(format!("invalid TOML: {e}")))?;
    manifest.dir = dir.to_path_buf();
    manifest.validate().map_err(LoadError::Invalid)?;
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_plugin(root: &Path, id: &str, body: &str) {
        let dir = root.join(id);
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(dir.join("plugin.toml"), body).expect("write manifest");
        std::fs::write(dir.join("init.lua"), b"return {}\n").expect("write entry");
    }

    fn good(id: &str) -> String {
        format!("id = \"{id}\"\nname = \"Good\"\nversion = \"1.0\"\napi = 1\n")
    }

    /// A store-installed plugin sits on a read-only path, so nothing may be
    /// written beside its manifest. Guards the Nix and Flatpak case.
    #[test]
    fn state_never_lands_next_to_the_manifest() {
        let root = std::env::temp_dir().join(format!("kopuz-ro-{}", uuid::Uuid::new_v4()));
        write_plugin(&root, "packaged", &good("packaged"));
        let Ok(manifest) = load(&root.join("packaged")) else {
            panic!("manifest loads");
        };
        assert!(
            !manifest.data_dir().starts_with(&manifest.dir),
            "data dir {} must not sit under the manifest dir {}",
            manifest.data_dir().display(),
            manifest.dir.display()
        );
        assert!(manifest.data_dir().ends_with("plugin-data/packaged"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn extra_roots_come_from_the_environment() {
        // Only the shape is asserted: the process env is shared across tests, so
        // setting KOPUZ_PLUGIN_PATH here would race.
        let roots = plugin_search_paths();
        assert_eq!(roots.first(), Some(&plugins_dir()));
    }

    #[test]
    fn id_charset() {
        assert!(is_valid_id("my-plugin_2"));
        assert!(!is_valid_id(""));
        assert!(!is_valid_id("My-Plugin"));
        assert!(!is_valid_id("a/b"));
        assert!(!is_valid_id("a:b"));
    }

    #[test]
    fn entry_must_stay_inside_the_plugin_dir() {
        assert!(is_contained_relative(Path::new("init.lua")));
        assert!(is_contained_relative(Path::new("lib/api.lua")));
        assert!(!is_contained_relative(Path::new("../escape.lua")));
        assert!(!is_contained_relative(Path::new("/etc/passwd")));
        assert!(!is_contained_relative(Path::new("")));
    }

    #[test]
    fn discovers_and_skips_broken() {
        let root = std::env::temp_dir().join(format!("kopuz-manifest-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("mkdir");

        write_plugin(&root, "good", &good("good"));
        // Rejected: the id is not in the allowed charset.
        write_plugin(&root, "bad-id", &good("Bad:Id"));
        // Rejected: the entry file is not there.
        write_plugin(
            &root,
            "no-entry",
            "id = \"no-entry\"\nname = \"No\"\nversion = \"1.0\"\napi = 1\nentry = \"missing.lua\"\n",
        );
        // Rejected: a future API generation this build cannot run.
        write_plugin(
            &root,
            "future",
            "id = \"future\"\nname = \"Future\"\nversion = \"1.0\"\napi = 99\n",
        );
        // Ignored: not a plugin directory at all.
        std::fs::create_dir_all(root.join("empty")).expect("mkdir");

        let found = discover_in(&root);
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(found.len(), 1, "only the valid manifest survives");
        assert_eq!(found[0].id, "good");
        assert_eq!(found[0].entry_path(), root.join("good").join("init.lua"));
    }

    #[test]
    fn missing_root_is_empty_not_an_error() {
        assert!(discover_in(Path::new("/nonexistent/kopuz/plugins")).is_empty());
    }
}
