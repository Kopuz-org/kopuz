//! Plugin discovery: `<config dir>/plugins/<id>/plugin.toml`.
//!
//! The manifest carries only what is needed to *list* a plugin without running
//! it — identity, where the binary is, and how to badge it in the UI. What the
//! plugin can actually do comes from the handshake ([`super::wire::InitializeResult`]),
//! so a manifest can never claim a capability the binary lacks.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Directory the host scans for plugins.
pub fn plugins_dir() -> PathBuf {
    db::config_dir().join("plugins")
}

/// The `plugin.toml` body, plus the directory it was found in.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PluginManifest {
    /// Stable identity. Namespaces the plugin's item ids in the database, so it
    /// is restricted to `[a-z0-9_-]+` — a `:` or `/` here would mis-slice every
    /// track ref downstream.
    pub id: String,
    /// Display name for the sidebar badge and settings row.
    pub name: String,
    pub version: String,
    /// Wire protocol the plugin speaks. Checked again at handshake.
    pub protocol: u32,
    /// The binary to run, relative to the manifest directory or absolute.
    pub executable: PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
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

impl PluginManifest {
    /// The executable resolved against the manifest directory.
    pub fn executable_path(&self) -> PathBuf {
        if self.executable.is_absolute() {
            self.executable.clone()
        } else {
            self.dir.join(&self.executable)
        }
    }

    /// This plugin's private state directory, handed to the child as
    /// `KOPUZ_PLUGIN_DATA_DIR`. Created by the host, owned by the plugin —
    /// never read or deleted by Kopuz, including when the user removes the
    /// source.
    pub fn data_dir(&self) -> PathBuf {
        self.dir.join("data")
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
        if self.executable.as_os_str().is_empty() {
            return Err("executable is empty".to_string());
        }
        let exe = self.executable_path();
        if !exe.is_file() {
            return Err(format!("executable {} does not exist", exe.display()));
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

/// Every readable manifest under [`plugins_dir`]. A bad entry is warned about
/// and skipped — one broken plugin must never hide the rest, and discovery runs
/// on a UI path where an `Err` has nowhere useful to go.
pub fn discover() -> Vec<PluginManifest> {
    discover_in(&plugins_dir())
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
    /// No `plugin.toml` here — an ordinary non-plugin directory.
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
        std::fs::write(dir.join("run"), b"#!/bin/sh\n").expect("write exe");
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
    fn discovers_and_skips_broken() {
        let root = std::env::temp_dir().join(format!("kopuz-manifest-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("mkdir");

        write_plugin(
            &root,
            "good",
            "id = \"good\"\nname = \"Good\"\nversion = \"1.0\"\nprotocol = 1\nexecutable = \"run\"\n",
        );
        // Rejected: the id is not in the allowed charset.
        write_plugin(
            &root,
            "bad-id",
            "id = \"Bad:Id\"\nname = \"Bad\"\nversion = \"1.0\"\nprotocol = 1\nexecutable = \"run\"\n",
        );
        // Rejected: the executable is not there.
        write_plugin(
            &root,
            "no-exe",
            "id = \"no-exe\"\nname = \"No\"\nversion = \"1.0\"\nprotocol = 1\nexecutable = \"missing\"\n",
        );
        // Ignored: not a plugin directory at all.
        std::fs::create_dir_all(root.join("empty")).expect("mkdir");

        let found = discover_in(&root);
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(found.len(), 1, "only the valid manifest survives");
        assert_eq!(found[0].id, "good");
        assert_eq!(found[0].executable_path(), root.join("good").join("run"));
    }

    #[test]
    fn missing_root_is_empty_not_an_error() {
        assert!(discover_in(Path::new("/nonexistent/kopuz/plugins")).is_empty());
    }
}
