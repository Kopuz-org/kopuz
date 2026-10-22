//! Plugins: the ones the daemon runs, and the community ones its marketplace
//! offers to install.

/// Who stands behind a plugin.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PluginOrigin {
    /// Shipped inside kopuz and updated with it. Never installed or removed.
    #[default]
    Official,
    /// Written by someone else and installed from the marketplace or by hand.
    Community,
}

/// One plugin, as installed, as the marketplace lists it, or both.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub author: String,
    pub homepage: Option<String>,
    pub origin: PluginOrigin,
    /// The version the daemon runs; `None` when it is not installed.
    pub installed_version: Option<String>,
    /// The version the marketplace offers; `None` when it lists none.
    pub available_version: Option<String>,
    /// The services it serves, by slug.
    pub services: Vec<String>,
    /// The hosts it may send requests to, as its manifest declares them;
    /// `$server` is the address of the server a source is configured with.
    pub network: Vec<String>,
}

impl PluginInfo {
    /// The marketplace offers a version other than the one running.
    pub fn update_available(&self) -> bool {
        match (&self.installed_version, &self.available_version) {
            (Some(installed), Some(available)) => installed != available,
            _ => false,
        }
    }
}

/// Every plugin the daemon knows of.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginCatalog {
    /// Installed ones first, then what the marketplace offers besides.
    pub plugins: Vec<PluginInfo>,
    /// Why the marketplace could not be read, when it could not; the
    /// installed plugins are listed regardless.
    pub marketplace_error: Option<String>,
}
