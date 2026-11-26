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
    /// From a Flatpak extension or a system package, whose package manager
    /// updates and removes it; the Plugins page cannot.
    pub system: bool,
    /// `None` when not installed.
    pub installed_version: Option<String>,
    /// `None` when the marketplace lists none.
    pub available_version: Option<String>,
    pub services: Vec<String>,
    /// `$server` is the configured server's host.
    pub network: Vec<String>,
    pub update: Option<PluginUpdate>,
    /// Reads the files in the folders picked for each of its sources.
    pub reads_folders: bool,
    /// Adds an integration, turned on under Connectivity, instead of a source.
    pub integration: bool,
    /// May play, pause, skip and change what is queued.
    pub controls_playback: bool,
    /// May read the library: its tracks, playlists and favorites.
    pub reads_library: bool,
}

/// A different build of an installed plugin: the marketplace lists another
/// version or another component, or new files were dropped into the plugin
/// directory and reach hosts the installed version does not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginUpdate {
    pub version: String,
    /// Everything it reaches. Installing it agrees to these.
    pub network: Vec<String>,
    /// What it reaches that the installed version does not, and as
    /// [`PLAYBACK_GRANT`] or [`LIBRARY_GRANT`] what more it may do.
    pub added_hosts: Vec<String>,
}

/// In [`PluginUpdate::added_hosts`], no host (unlike `$server`, which is one): the update newly asks to play,
/// pause, skip and queue.
pub const PLAYBACK_GRANT: &str = "$playback:control";
/// In [`PluginUpdate::added_hosts`], no host: the update newly asks to read the library.
pub const LIBRARY_GRANT: &str = "$library:read";

impl PluginUpdate {
    /// The hosts it reaches that the installed version does not.
    pub fn new_hosts(&self) -> Vec<String> {
        self.added_hosts
            .iter()
            .filter(|host| *host != PLAYBACK_GRANT && *host != LIBRARY_GRANT)
            .cloned()
            .collect()
    }

    /// Whether it asks for `grant`, one of [`PLAYBACK_GRANT`] and [`LIBRARY_GRANT`], anew.
    pub fn adds(&self, grant: &str) -> bool {
        self.added_hosts.iter().any(|added| added == grant)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginCatalog {
    pub plugins: Vec<PluginInfo>,
    pub marketplace_error: Option<String>,
}

/// Where the app offers a plugin's action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionPlace {
    /// The command palette, run on nothing in particular.
    Palette,
    /// A track's menu and a selection's, run on their tracks.
    Track,
    /// An album's menu, run on the album.
    Album,
    /// An artist's menu, run on the artist.
    Artist,
}

/// Something an integration plugin that is turned on offers to run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginAction {
    pub plugin: String,
    pub id: String,
    /// In the plugin's own words.
    pub label: String,
    pub places: Vec<ActionPlace>,
    /// Asked of the person before it runs; their answer is its input.
    pub ask: Option<String>,
}

/// What an action is run on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ActionTarget {
    #[default]
    None,
    Tracks(Vec<String>),
    Album(String),
    /// An artist's library key.
    Artist(String),
}

/// Where a plugin's panel is drawn: at the bottom of these pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelPlace {
    Album,
    Artist,
}

/// A section an integration plugin that is turned on fills.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginPanel {
    pub plugin: String,
    pub id: String,
    /// In the plugin's own words.
    pub title: String,
    pub places: Vec<PanelPlace>,
}

/// What a panel is drawn for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelTarget {
    Album(String),
    /// An artist's library key.
    Artist(String),
}

/// One piece of a panel, drawn by the app in its own style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelBlock {
    Heading(String),
    Text(String),
    /// Library track keys, drawn as rows that play from where they are clicked.
    Tracks(Vec<String>),
    /// A button that runs the plugin's action `action` on what the panel is for.
    Action {
        label: String,
        action: String,
    },
}

#[cfg(test)]
mod update_tests {
    use super::*;

    #[test]
    fn an_update_tells_its_new_hosts_from_its_new_grants() {
        let update = PluginUpdate {
            added_hosts: vec!["$server".into(), "b.test".into(), PLAYBACK_GRANT.into()],
            ..PluginUpdate::default()
        };
        assert_eq!(update.new_hosts(), ["$server", "b.test"]);
        assert!(update.adds(PLAYBACK_GRANT));
        assert!(!update.adds(LIBRARY_GRANT));
    }
}
