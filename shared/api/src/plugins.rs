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
    pub update: Option<PluginUpdate>,
}

/// A different build of an installed plugin: the marketplace lists another
/// version or another component, or new files were dropped into the plugin
/// directory and reach hosts the installed version does not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginUpdate {
    pub version: String,
    /// Everything it reaches. Installing it agrees to these.
    pub network: Vec<String>,
    /// What it reaches that the installed version does not.
    pub added_hosts: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginCatalog {
    pub plugins: Vec<PluginInfo>,
    pub marketplace_error: Option<String>,
}
