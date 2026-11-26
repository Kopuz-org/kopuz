//! `kopuz-plugin.toml`: what a plugin is, what it may reach and what each of
//! its services supports.

use serde::Deserialize;

/// 0.2 added the `tcp`, `browser` and `decryption` imports and the browser
/// sign-in; a 0.1 plugin runs unchanged.
pub const SUPPORTED_API: &[&str] = &["0.1", "0.2"];

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
    /// What a plugin that is not a source adds. A plugin is one or the other.
    #[serde(default)]
    pub integration: Option<Integration>,
}

/// A plugin that is not a source, set up from the fields it lists.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Integration {
    /// The `url` field whose host `$server` names. Like a source's server, it
    /// may be on this machine or its network, since the person typed it.
    #[serde(default)]
    pub server: Option<String>,
    /// An SVG path in a 24x24 box.
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default, rename = "field")]
    pub fields: Vec<Field>,
}

/// One thing the person fills in, in the order the settings page shows it.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Field {
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub kind: FieldKind,
    #[serde(default)]
    pub help: Option<String>,
    #[serde(default)]
    pub placeholder: Option<String>,
    /// What a new setup starts with; never shown for a secret.
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub required: bool,
    /// The values a `choice` field takes.
    #[serde(default)]
    pub options: Vec<FieldOption>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum FieldKind {
    #[default]
    Text,
    /// Kept with the daemon's credentials and never sent back out.
    Secret,
    Url,
    Toggle,
    Choice,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FieldOption {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Permissions {
    /// `$server` is the configured server's host; `*.example.com` also
    /// matches `example.com`.
    #[serde(default)]
    pub network: Vec<String>,
    /// The folders the person picked for a source, read-only. The Plugins page
    /// warns before such a plugin is installed.
    #[serde(default)]
    pub folders: FolderAccess,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum FolderAccess {
    #[default]
    None,
    Read,
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
    /// The plugin's `login` runs with the `browser` import answering, and
    /// with no username or password.
    Browser,
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
    #[error(
        "the manifest lists `{0}` as a host, but it is an address; name the configured server as `$server`"
    )]
    NamesAddress(String),
    #[error("a plugin is a source or an integration, and this manifest lists both")]
    SourceAndIntegration,
    #[error("the integration lists field `{0}` twice")]
    DuplicateField(String),
    #[error("the integration's server is `{0}`, which is not one of its url fields")]
    NoServerField(String),
    #[error("the integration's choice field `{0}` has no options")]
    NoOptions(String),
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
        for host in &manifest.permissions.network {
            if let Some(address) = address_literal(host) {
                return Err(ManifestError::NamesAddress(address));
            }
        }
        let mut seen = std::collections::HashSet::new();
        for service in &manifest.services {
            if !seen.insert(service.id.as_str()) {
                return Err(ManifestError::DuplicateService(service.id.clone()));
            }
        }
        if let Some(integration) = &manifest.integration {
            if !manifest.services.is_empty() {
                return Err(ManifestError::SourceAndIntegration);
            }
            integration.check()?;
        }
        Ok(manifest)
    }

    pub fn is_integration(&self) -> bool {
        self.integration.is_some()
    }

    /// Whether a stream this plugin resolves may be one the host decrypts.
    pub fn may_encrypt_streams(&self) -> bool {
        self.api != "0.1"
    }

    pub fn service(&self, id: &str) -> Option<&Service> {
        self.services.iter().find(|service| service.id == id)
    }
}

impl Integration {
    fn check(&self) -> Result<(), ManifestError> {
        let mut seen = std::collections::HashSet::new();
        for field in &self.fields {
            if !seen.insert(field.key.as_str()) {
                return Err(ManifestError::DuplicateField(field.key.clone()));
            }
            if field.kind == FieldKind::Choice && field.options.is_empty() {
                return Err(ManifestError::NoOptions(field.key.clone()));
            }
        }
        if let Some(server) = &self.server
            && self
                .field(server)
                .is_none_or(|field| field.kind != FieldKind::Url)
        {
            return Err(ManifestError::NoServerField(server.clone()));
        }
        Ok(())
    }

    pub fn field(&self, key: &str) -> Option<&Field> {
        self.fields.iter().find(|field| field.key == key)
    }

    /// The address `$server` names for these answers, if the integration has one.
    pub fn server_url<'a>(&self, settings: &'a [(String, String)]) -> &'a str {
        self.server
            .as_deref()
            .and_then(|server| {
                settings
                    .iter()
                    .find(|(key, _)| key == server)
                    .map(|(_, value)| value.as_str())
            })
            .unwrap_or_default()
    }
}

/// `$server` names the configured server, whatever its address. A host the
/// manifest lists is checked where it resolves; an address is not resolved, so
/// one listed here would reach this machine or its network unchecked.
fn address_literal(host: &str) -> Option<String> {
    let bare = host
        .trim()
        .trim_end_matches('.')
        .trim_start_matches('[')
        .trim_end_matches(']');
    bare.parse::<std::net::IpAddr>()
        .ok()
        .map(|_| host.trim().to_string())
}

impl Permissions {
    pub fn reads_folders(&self) -> bool {
        self.folders == FolderAccess::Read
    }

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
        assert!(!manifest.may_encrypt_streams());
        let newer = SUBSONIC
            .replace("api = \"0.1\"", "api = \"0.2\"")
            .replace("sign-in = \"password\"", "sign-in = \"browser\"");
        let newer = Manifest::parse(&newer).expect("valid manifest");
        assert!(newer.may_encrypt_streams());
        assert_eq!(
            newer.service("custom").map(|custom| custom.sign_in),
            Some(SignIn::Browser)
        );
        assert!(!manifest.permissions.reads_folders());
    }

    #[test]
    fn folder_access_is_read_or_nothing() {
        let reading = SUBSONIC.replace(
            "network = [\"$server\"]",
            "network = []\nfolders = \"read\"",
        );
        let manifest = Manifest::parse(&reading).expect("valid manifest");
        assert!(manifest.permissions.reads_folders());
        let writing = SUBSONIC.replace(
            "network = [\"$server\"]",
            "network = []\nfolders = \"write\"",
        );
        assert!(Manifest::parse(&writing).is_err());
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
            ..Permissions::default()
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

    #[test]
    fn a_listed_host_may_not_be_an_address() {
        for address in ["127.0.0.1", "169.254.169.254.", "[::1]", "::1", "0.0.0.0"] {
            let manifest = SUBSONIC.replace(
                "network = [\"$server\"]",
                &format!("network = [\"{address}\"]"),
            );
            assert!(
                matches!(
                    Manifest::parse(&manifest),
                    Err(ManifestError::NamesAddress(_))
                ),
                "{address} is an address, not a host"
            );
        }
        let named = SUBSONIC.replace(
            "network = [\"$server\"]",
            "network = [\"$server\", \"*.sndcdn.com\"]",
        );
        assert!(Manifest::parse(&named).is_ok());
    }

    const LIGHTS: &str = r#"
        id = "lights"
        name = "Lights"
        version = "0.1.0"
        api = "0.2"

        [permissions]
        network = ["$server"]

        [integration]
        server = "url"

        [[integration.field]]
        key = "url"
        label = "Address"
        kind = "url"
        default = "ws://127.0.0.1:12345"
        required = true

        [[integration.field]]
        key = "mode"
        label = "Follow"
        kind = "choice"
        options = [{ value = "beat", label = "The beat" }, { value = "level", label = "The loudness" }]
    "#;

    #[test]
    fn an_integration_reads_its_fields_and_names_its_server() {
        let manifest = Manifest::parse(LIGHTS).expect("an integration manifest");
        assert!(manifest.is_integration());
        let integration = manifest.integration.expect("integration");
        assert_eq!(integration.fields.len(), 2);
        assert_eq!(integration.fields[0].kind, FieldKind::Url);
        assert_eq!(integration.fields[1].options[1].value, "level");
        let answers = vec![
            ("mode".to_string(), "beat".to_string()),
            ("url".to_string(), "ws://127.0.0.1:12345".to_string()),
        ];
        assert_eq!(integration.server_url(&answers), "ws://127.0.0.1:12345");
        assert_eq!(integration.server_url(&[]), "");
    }

    #[test]
    fn a_source_manifest_is_no_integration() {
        assert!(
            !Manifest::parse(SUBSONIC)
                .expect("subsonic")
                .is_integration()
        );
    }

    #[test]
    fn an_integration_is_refused_where_its_fields_do_not_add_up() {
        let both = format!("{LIGHTS}\n[[service]]\nid = \"lights\"\n");
        assert!(matches!(
            Manifest::parse(&both),
            Err(ManifestError::SourceAndIntegration)
        ));
        let twice = LIGHTS.replace("key = \"mode\"", "key = \"url\"");
        assert!(matches!(
            Manifest::parse(&twice),
            Err(ManifestError::DuplicateField(key)) if key == "url"
        ));
        let no_server = LIGHTS.replace("server = \"url\"", "server = \"mode\"");
        assert!(matches!(
            Manifest::parse(&no_server),
            Err(ManifestError::NoServerField(key)) if key == "mode"
        ));
        let no_options = LIGHTS.replace(
            "options = [{ value = \"beat\", label = \"The beat\" }, { value = \"level\", label = \"The loudness\" }]",
            "",
        );
        assert!(matches!(
            Manifest::parse(&no_options),
            Err(ManifestError::NoOptions(key)) if key == "mode"
        ));
        let unknown = LIGHTS.replace("required = true", "required = true\nsecret = true");
        assert!(matches!(
            Manifest::parse(&unknown),
            Err(ManifestError::Parse(_))
        ));
    }
}
