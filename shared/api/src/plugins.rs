//! Plugins: the ones the daemon runs, and what its marketplace offers.

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PluginOrigin {
    /// Ships inside kopuz; never installed or removed.
    #[default]
    Official,
    Community,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub author: String,
    pub homepage: Option<String>,
    pub origin: PluginOrigin,
    /// False for an official plugin that runs in-process.
    pub sandboxed: bool,
    /// `None` when not installed.
    pub installed_version: Option<String>,
    /// `None` when the marketplace lists none.
    pub available_version: Option<String>,
    pub services: Vec<String>,
    /// `$server` is the configured server's host.
    pub network: Vec<String>,
}

impl PluginInfo {
    pub fn update_available(&self) -> bool {
        match (&self.installed_version, &self.available_version) {
            (Some(installed), Some(available)) => installed != available,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginCatalog {
    pub plugins: Vec<PluginInfo>,
    pub marketplace_error: Option<String>,
}
