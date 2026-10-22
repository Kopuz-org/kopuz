//! `kopuz-plugin.toml`: what a plugin is, what it may reach and what each of
//! its services supports.

use serde::Deserialize;

pub const SUPPORTED_API: &[&str] = &["0.1"];

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// An installed plugin with a bundled plugin's id replaces it.
    pub id: String,
    pub name: String,
    pub version: String,
    pub api: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub permissions: Permissions,
    #[serde(default, rename = "service")]
    pub services: Vec<Service>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Permissions {
    /// `$server` is the configured server's host; `*.example.com` also
    /// matches `example.com`.
    #[serde(default)]
    pub network: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Service {
    pub id: String,
    #[serde(default)]
    pub capabilities: Capabilities,
    /// How a service of the plugin's own is shown; a built-in one keeps the
    /// host's.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    /// An SVG path in a 24x24 box.
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub accent: Option<String>,
    #[serde(default)]
    pub sign_in: SignIn,
    /// A hosted service's one address; none asks for one.
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SignIn {
    #[default]
    None,
    /// Exchanged by the plugin's `login`.
    Password,
}

/// Every flag defaults to off.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Capabilities {
    pub edit_tags: bool,
    pub delete_from_disk: bool,
    pub scan_folders: bool,
    pub folders: bool,
    pub browse_folders: bool,
    pub external_devices: bool,
    pub browser_playback: bool,
    pub accounts: bool,
    pub sync: bool,
    pub downloads: bool,
    pub uploads: bool,
    pub storage_quota: bool,
    pub discover: bool,
    pub dont_recommend: bool,
    pub music_videos: bool,
    pub radio: Vec<RadioSeed>,
    pub play_queue: bool,
    pub playlists: PlaylistOps,
    pub artist_view: ArtistView,
    pub favorites_sync: FavoritesSync,
    pub account_avatar: bool,
    pub stream_quality: bool,
    pub explicit_flags: bool,
    pub watch_history: bool,
    pub complete_play_history: bool,
    pub library_actions: Vec<LibraryAction>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RadioSeed {
    Track,
    Playlist,
    Search,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PlaylistOps {
    #[default]
    None,
    AddRemove,
    Reorder,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ArtistView {
    #[default]
    Library,
    Remote,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum FavoritesSync {
    #[default]
    Instant,
    Paginated,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum LibraryAction {
    Rate,
    Follow,
    Save,
    RemoveFromHistory,
    PlaylistDetails,
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("the manifest is not valid: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("the plugin targets API {0}, which this version of kopuz does not run")]
    UnsupportedApi(String),
    #[error("the manifest has no id")]
    MissingId,
    #[error("the manifest lists service `{0}` twice")]
    DuplicateService(String),
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Self, ManifestError> {
        let manifest: Manifest = toml::from_str(text)?;
        if manifest.id.trim().is_empty() {
            return Err(ManifestError::MissingId);
        }
        if !SUPPORTED_API.contains(&manifest.api.as_str()) {
            return Err(ManifestError::UnsupportedApi(manifest.api));
        }
        let mut seen = std::collections::HashSet::new();
        for service in &manifest.services {
            if !seen.insert(service.id.as_str()) {
                return Err(ManifestError::DuplicateService(service.id.clone()));
            }
        }
        Ok(manifest)
    }

    pub fn service(&self, id: &str) -> Option<&Service> {
        self.services.iter().find(|service| service.id == id)
    }
}

impl Permissions {
    pub fn allows(&self, host: &str, server_host: Option<&str>) -> bool {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        self.network.iter().any(|pattern| {
            let pattern = pattern.trim().to_ascii_lowercase();
            if pattern == "$server" {
                return server_host.is_some_and(|server| server.eq_ignore_ascii_case(&host));
            }
            match pattern.strip_prefix("*.") {
                Some(domain) => {
                    host == domain
                        || host
                            .strip_suffix(domain)
                            .is_some_and(|rest| rest.ends_with('.'))
                }
                None => host == pattern,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUBSONIC: &str = r#"
        id = "subsonic"
        name = "Subsonic"
        version = "0.1.0"
        api = "0.1"

        [permissions]
        network = ["$server"]

        [[service]]
        id = "subsonic"
        capabilities = { sync = true, radio = ["track"], playlists = "reorder" }

        [[service]]
        id = "custom"
        sign-in = "password"
    "#;

    #[test]
    fn a_manifest_reads_its_services_and_their_capabilities() {
        let manifest = Manifest::parse(SUBSONIC).expect("valid manifest");
        let subsonic = manifest.service("subsonic").expect("subsonic listed");
        assert!(subsonic.capabilities.sync);
        assert_eq!(subsonic.capabilities.radio, [RadioSeed::Track]);
        assert_eq!(subsonic.capabilities.playlists, PlaylistOps::Reorder);
        let custom = manifest.service("custom").expect("custom listed");
        assert_eq!(custom.capabilities, Capabilities::default());
        assert_eq!(custom.sign_in, SignIn::Password);
        assert_eq!(subsonic.sign_in, SignIn::None);
    }

    #[test]
    fn an_unknown_api_or_a_repeated_service_is_refused() {
        let future = SUBSONIC.replace("api = \"0.1\"", "api = \"9.0\"");
        assert!(matches!(
            Manifest::parse(&future),
            Err(ManifestError::UnsupportedApi(api)) if api == "9.0"
        ));
        let repeated = SUBSONIC.replace("id = \"custom\"", "id = \"subsonic\"");
        assert!(matches!(
            Manifest::parse(&repeated),
            Err(ManifestError::DuplicateService(id)) if id == "subsonic"
        ));
    }

    #[test]
    fn a_misspelled_capability_is_an_error_not_a_silent_off() {
        let typo = SUBSONIC.replace("sync = true", "synk = true");
        assert!(matches!(
            Manifest::parse(&typo),
            Err(ManifestError::Parse(_))
        ));
    }

    #[test]
    fn network_patterns_match_the_server_and_subdomains_only() {
        let permissions = Permissions {
            network: vec![
                "$server".into(),
                "*.sndcdn.com".into(),
                "api.example.com".into(),
            ],
        };
        assert!(permissions.allows("music.home.test", Some("music.home.test")));
        assert!(!permissions.allows("music.home.test", None));
        assert!(!permissions.allows("evil.test", Some("music.home.test")));
        assert!(permissions.allows("cf-media.sndcdn.com", None));
        assert!(permissions.allows("sndcdn.com", None));
        assert!(!permissions.allows("notsndcdn.com", None));
        assert!(permissions.allows("API.example.com.", None));
        assert!(!permissions.allows("www.api.example.com", None));
    }
}
