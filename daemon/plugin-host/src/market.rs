//! The community marketplace: a JSON index naming each plugin's manifest and
//! component and the SHA-256 of both. Nothing is trusted beyond those hashes,
//! and the manifest is checked again before anything is written. A listing
//! may not claim an official plugin's id or a built-in service.
//!
//! A listing marked verified was reviewed by the kopuz maintainers, who set
//! the mark by hand in the index. Only such a build may read the folders a
//! person picks for its sources: installing it records the hashes of its
//! files in [`VERIFIED_FILE`], and a plugin asking for folders loads only
//! while its files still match a record.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::manifest::{FolderAccess, Manifest, ManifestError};
use crate::registry::{COMPONENT_FILE, MANIFEST_FILE};

/// The builds installed from a verified listing, kept in the plugin directory.
pub const VERIFIED_FILE: &str = ".verified.toml";

/// Plugin ids a debug build loads with folder access unverified, comma
/// separated, for trying a plugin under development. Release builds ignore it.
pub const TRUST_ENV: &str = "KOPUZ_TRUST_PLUGINS";

pub const INDEX_SCHEMA: u32 = 1;

pub const MAX_COMPONENT_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Index {
    pub schema: u32,
    #[serde(default)]
    pub plugins: Vec<Listing>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Listing {
    pub id: String,
    pub name: String,
    pub version: String,
    pub api: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub services: Vec<String>,
    /// Shown before install; a manifest asking for anything else is refused.
    #[serde(default)]
    pub network: Vec<String>,
    /// Shown before install, like `network`.
    #[serde(default)]
    pub folders: FolderAccess,
    /// Reviewed by the kopuz maintainers, the only build that may read folders.
    #[serde(default)]
    pub verified: bool,
    pub manifest: Download,
    pub component: Download,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Download {
    pub url: String,
    /// Lowercase hex.
    pub sha256: String,
}

#[derive(Debug, thiserror::Error)]
pub enum MarketError {
    #[error("the marketplace index is not valid: {0}")]
    Index(String),
    #[error("the marketplace index is format {0}, which this version of kopuz does not read")]
    IndexSchema(u32),
    #[error("the marketplace lists no plugin `{0}`")]
    NotListed(String),
    #[error("`{0}` is an official plugin; it ships with kopuz and is updated with it")]
    Official(String),
    #[error("`{0}` comes from a Flatpak extension or a system package; remove it there")]
    Extension(String),
    #[error("the download does not match the hash the marketplace lists")]
    HashMismatch,
    #[error("the download is larger than a plugin may be")]
    TooLarge,
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error("the plugin's manifest says it is `{found}`, not `{listed}`")]
    WrongPlugin { listed: String, found: String },
    #[error("the plugin's manifest is version {found}, not the {listed} the marketplace lists")]
    WrongVersion { listed: String, found: String },
    #[error("the plugin claims `{0}`, a service kopuz serves itself")]
    ClaimsBuiltIn(String),
    #[error("`{service}` is already served by the installed plugin `{by}`; remove it first")]
    ServiceTaken { service: String, by: String },
    #[error("the plugin's manifest asks for more than the marketplace lists")]
    PermissionsDiffer,
    #[error("`{0}` asks to read your folders, which only a verified plugin may do")]
    Unverified(String),
    #[error("could not write the plugin: {0}")]
    Io(#[from] std::io::Error),
}

impl Index {
    pub fn parse(json: &[u8]) -> Result<Self, MarketError> {
        let index: Index =
            serde_json::from_slice(json).map_err(|error| MarketError::Index(error.to_string()))?;
        if index.schema != INDEX_SCHEMA {
            return Err(MarketError::IndexSchema(index.schema));
        }
        Ok(index)
    }

    pub fn listing(&self, id: &str) -> Result<&Listing, MarketError> {
        self.plugins
            .iter()
            .find(|listing| listing.id == id)
            .ok_or_else(|| MarketError::NotListed(id.to_string()))
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn verify(download: &Download, bytes: &[u8]) -> Result<(), MarketError> {
    if sha256_hex(bytes).eq_ignore_ascii_case(download.sha256.trim()) {
        Ok(())
    } else {
        Err(MarketError::HashMismatch)
    }
}

pub fn check_listing(listing: &Listing, official_ids: &[String]) -> Result<(), MarketError> {
    if official_ids.contains(&listing.id) {
        return Err(MarketError::Official(listing.id.clone()));
    }
    if !crate::manifest::SUPPORTED_API.contains(&listing.api.as_str()) {
        return Err(MarketError::Manifest(ManifestError::UnsupportedApi(
            listing.api.clone(),
        )));
    }
    Ok(())
}

/// A plugin taking a service another installed plugin serves would be handed
/// that plugin's sources and their credentials. `serving` names the plugin
/// serving a service now.
pub fn check_services(
    listing: &Listing,
    serving: impl Fn(&str) -> Option<String>,
) -> Result<(), MarketError> {
    for service in &listing.services {
        if let Some(by) = serving(service).filter(|by| *by != listing.id) {
            return Err(MarketError::ServiceTaken {
                service: service.clone(),
                by,
            });
        }
    }
    Ok(())
}

/// A community plugin claiming a built-in service would be handed that
/// service's credentials.
pub fn check_manifest(
    listing: &Listing,
    manifest_text: &str,
    built_in_services: &[&str],
) -> Result<Manifest, MarketError> {
    let manifest = Manifest::parse(manifest_text)?;
    if manifest.id != listing.id {
        return Err(MarketError::WrongPlugin {
            listed: listing.id.clone(),
            found: manifest.id,
        });
    }
    if manifest.version != listing.version {
        return Err(MarketError::WrongVersion {
            listed: listing.version.clone(),
            found: manifest.version,
        });
    }
    if let Some(claimed) = manifest
        .services
        .iter()
        .find(|service| built_in_services.contains(&service.id.as_str()))
    {
        return Err(MarketError::ClaimsBuiltIn(claimed.id.clone()));
    }
    let declared: std::collections::BTreeSet<&str> =
        listing.network.iter().map(String::as_str).collect();
    let asked: std::collections::BTreeSet<&str> = manifest
        .permissions
        .network
        .iter()
        .map(String::as_str)
        .collect();
    let services: std::collections::BTreeSet<&str> = manifest
        .services
        .iter()
        .map(|service| service.id.as_str())
        .collect();
    let listed_services: std::collections::BTreeSet<&str> =
        listing.services.iter().map(String::as_str).collect();
    if declared != asked
        || services != listed_services
        || manifest.permissions.folders != listing.folders
    {
        return Err(MarketError::PermissionsDiffer);
    }
    if manifest.permissions.reads_folders() && !listing.verified {
        return Err(MarketError::Unverified(listing.id.clone()));
    }
    Ok(manifest)
}

/// The hashes of one build installed from a verified listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedBuild {
    pub manifest: String,
    pub component: String,
}

impl VerifiedBuild {
    pub fn of(manifest_text: &str, component: &[u8]) -> Self {
        Self {
            manifest: sha256_hex(manifest_text.as_bytes()),
            component: sha256_hex(component),
        }
    }
}

pub type Verified = BTreeMap<String, VerifiedBuild>;

/// What [`VERIFIED_FILE`] in `dir` records; nothing when it is missing or
/// unreadable, which leaves every plugin unverified.
pub fn read_verified(dir: &Path) -> Verified {
    let text = match std::fs::read_to_string(dir.join(VERIFIED_FILE)) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Verified::new(),
        Err(error) => {
            tracing::warn!(dir = %dir.display(), %error, "verified plugin record unreadable");
            return Verified::new();
        }
    };
    toml::from_str(&text).unwrap_or_else(|error| {
        tracing::warn!(dir = %dir.display(), %error, "verified plugin record unreadable");
        Verified::new()
    })
}

/// Record `build` as the verified one for `id`, or forget `id` with `None`.
/// Written through a rename so a crash never leaves half a record. Hold
/// [`lock`] around it.
pub fn set_verified(dir: &Path, id: &str, build: Option<VerifiedBuild>) -> std::io::Result<()> {
    let mut verified = read_verified(dir);
    let changed = match build {
        Some(build) => verified.insert(id.to_string(), build.clone()) != Some(build),
        None => verified.remove(id).is_some(),
    };
    if !changed {
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    let text = toml::to_string(&verified).map_err(std::io::Error::other)?;
    let staging = dir.join(format!("{VERIFIED_FILE}.writing"));
    std::fs::write(&staging, text)?;
    std::fs::rename(&staging, dir.join(VERIFIED_FILE))
}

/// Whether a plugin read from files whose contents are `manifest_text` and
/// `component` may load with what its manifest asks for: one that reads
/// folders needs a verified record of these very files.
pub fn may_load(
    verified: &Verified,
    manifest: &Manifest,
    manifest_text: &str,
    component: &[u8],
) -> Result<(), String> {
    if !manifest.permissions.reads_folders() {
        return Ok(());
    }
    if trusted_for_development(&manifest.id) {
        tracing::warn!(plugin = %manifest.id, "loading a plugin that reads folders unverified, as {TRUST_ENV} asks");
        return Ok(());
    }
    match verified.get(&manifest.id) {
        Some(build) if *build == VerifiedBuild::of(manifest_text, component) => Ok(()),
        Some(_) => Err(
            "it reads folders, and its files differ from the verified build installed from the marketplace"
                .into(),
        ),
        None => Err(
            "it reads folders, which only a plugin installed from a verified marketplace listing may do"
                .into(),
        ),
    }
}

fn trusted_for_development(id: &str) -> bool {
    cfg!(debug_assertions)
        && std::env::var(TRUST_ENV)
            .is_ok_and(|ids| ids.split(',').any(|trusted| trusted.trim() == id))
}

/// An advisory lock on `dir`, released when the file is dropped. Hold it
/// across an install or removal, so two daemons sharing the directory never
/// stage, publish or remove one plugin at once.
pub fn lock(dir: &Path) -> Result<std::fs::File, MarketError> {
    std::fs::create_dir_all(dir)?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(LOCK_FILE))?;
    file.lock()?;
    Ok(file)
}

const LOCK_FILE: &str = ".lock";

/// Staged first, so a failure halfway leaves the old version whole.
pub fn install(
    dir: &Path,
    id: &str,
    manifest: &str,
    component: &[u8],
) -> Result<PathBuf, MarketError> {
    let target = plugin_dir(dir, id)?;
    std::fs::create_dir_all(dir)?;
    let staging = dir.join(format!(".{id}.installing"));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;
    std::fs::write(staging.join(MANIFEST_FILE), manifest)?;
    std::fs::write(staging.join(COMPONENT_FILE), component)?;
    let retired = dir.join(format!(".{id}.removing"));
    let _ = std::fs::remove_dir_all(&retired);
    if target.exists() {
        std::fs::rename(&target, &retired)?;
    }
    if let Err(error) = std::fs::rename(&staging, &target) {
        let _ = std::fs::rename(&retired, &target);
        return Err(error.into());
    }
    let _ = std::fs::remove_dir_all(&retired);
    Ok(target)
}

/// Remove the plugin directory `target`, which must sit directly under `dir`;
/// its name need not be the plugin's id.
pub fn uninstall(dir: &Path, target: &Path) -> Result<(), MarketError> {
    if target.parent() != Some(dir) || target.file_name().is_none() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("`{}` is not in the plugin directory", target.display()),
        )
        .into());
    }
    match std::fs::remove_dir_all(target) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn plugin_dir(dir: &Path, id: &str) -> Result<PathBuf, MarketError> {
    let valid = !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        && !id.starts_with(['-', '_']);
    if !valid {
        return Err(MarketError::Index(format!(
            "`{id}` is not a valid plugin id"
        )));
    }
    Ok(dir.join(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_directory_lock_keeps_a_second_holder_out_until_it_is_dropped() {
        let dir = std::env::temp_dir().join(format!("kopuz-plugin-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let held = lock(&dir).expect("lock");
        let other = std::fs::File::open(dir.join(LOCK_FILE)).expect("open");
        assert!(matches!(
            other.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
        drop(held);
        assert!(other.try_lock().is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn sha(bytes: &[u8]) -> String {
        sha256_hex(bytes)
    }

    const MANIFEST: &str = r#"
        id = "bandcamp"
        name = "Bandcamp"
        version = "0.2.0"
        api = "0.1"

        [permissions]
        network = ["*.bandcamp.com"]

        [[service]]
        id = "bandcamp"
        name = "Bandcamp"
    "#;

    fn listing() -> Listing {
        Listing {
            id: "bandcamp".into(),
            name: "Bandcamp".into(),
            version: "0.2.0".into(),
            api: "0.1".into(),
            description: String::new(),
            author: "someone".into(),
            homepage: None,
            services: vec!["bandcamp".into()],
            network: vec!["*.bandcamp.com".into()],
            folders: FolderAccess::None,
            verified: false,
            manifest: Download {
                url: "https://example.test/kopuz-plugin.toml".into(),
                sha256: sha(MANIFEST.as_bytes()),
            },
            component: Download {
                url: "https://example.test/plugin.wasm".into(),
                sha256: sha(b"wasm"),
            },
        }
    }

    #[test]
    fn an_index_reads_its_listings_and_refuses_another_format() {
        let json = br#"{"schema":1,"plugins":[{"id":"bandcamp","name":"Bandcamp","version":"0.2.0","api":"0.1",
            "manifest":{"url":"https://x/m","sha256":"00"},"component":{"url":"https://x/c","sha256":"11"}}]}"#;
        let index = Index::parse(json).expect("valid index");
        assert_eq!(index.listing("bandcamp").unwrap().version, "0.2.0");
        assert!(matches!(
            index.listing("nope"),
            Err(MarketError::NotListed(_))
        ));
        assert!(matches!(
            Index::parse(br#"{"schema":2}"#),
            Err(MarketError::IndexSchema(2))
        ));
    }

    #[test]
    fn a_download_must_match_its_hash() {
        let listing = listing();
        assert!(verify(&listing.component, b"wasm").is_ok());
        assert!(matches!(
            verify(&listing.component, b"tampered"),
            Err(MarketError::HashMismatch)
        ));
    }

    #[test]
    fn a_listing_cannot_stand_in_for_an_official_plugin() {
        let mut listing = listing();
        let official = ["subsonic".to_string(), "jellyfin".to_string()];
        assert!(check_listing(&listing, &official).is_ok());
        listing.id = "subsonic".into();
        assert!(matches!(
            check_listing(&listing, &official),
            Err(MarketError::Official(_))
        ));
    }

    #[test]
    fn a_listing_cannot_take_a_service_another_plugin_serves() {
        let listing = listing();
        assert!(check_services(&listing, |_| None).is_ok());
        assert!(check_services(&listing, |_| Some("bandcamp".into())).is_ok());
        assert!(matches!(
            check_services(&listing, |_| Some("other".into())),
            Err(MarketError::ServiceTaken { service, by }) if service == "bandcamp" && by == "other"
        ));
    }

    #[test]
    fn the_manifest_must_be_the_listed_plugin_and_claim_no_built_in_service() {
        let listing = listing();
        assert!(check_manifest(&listing, MANIFEST, &["jellyfin"]).is_ok());

        let other = MANIFEST.replacen("id = \"bandcamp\"", "id = \"other\"", 1);
        assert!(matches!(
            check_manifest(&listing, &other, &[]),
            Err(MarketError::WrongPlugin { .. })
        ));
        let older = MANIFEST.replace("0.2.0", "0.1.0");
        assert!(matches!(
            check_manifest(&listing, &older, &[]),
            Err(MarketError::WrongVersion { .. })
        ));
        let wider = MANIFEST.replace(
            "network = [\"*.bandcamp.com\"]",
            "network = [\"*.bandcamp.com\", \"evil.test\"]",
        );
        assert!(matches!(
            check_manifest(&listing, &wider, &[]),
            Err(MarketError::PermissionsDiffer)
        ));
        let hijack = MANIFEST.replace(
            "[[service]]\n        id = \"bandcamp\"",
            "[[service]]\n        id = \"jellyfin\"",
        );
        assert!(matches!(
            check_manifest(&listing, &hijack, &["jellyfin"]),
            Err(MarketError::ClaimsBuiltIn(service)) if service == "jellyfin"
        ));
    }

    fn reading_folders(manifest: &str) -> String {
        manifest.replace(
            "network = [\"*.bandcamp.com\"]",
            "network = [\"*.bandcamp.com\"]\n        folders = \"read\"",
        )
    }

    #[test]
    fn only_a_verified_listing_may_read_folders_and_it_must_say_so() {
        let reads = reading_folders(MANIFEST);
        let mut listing = listing();
        assert!(matches!(
            check_manifest(&listing, &reads, &[]),
            Err(MarketError::PermissionsDiffer)
        ));
        listing.folders = FolderAccess::Read;
        assert!(matches!(
            check_manifest(&listing, &reads, &[]),
            Err(MarketError::Unverified(id)) if id == "bandcamp"
        ));
        listing.verified = true;
        assert!(check_manifest(&listing, &reads, &[]).is_ok());
        assert!(matches!(
            check_manifest(&listing, MANIFEST, &[]),
            Err(MarketError::PermissionsDiffer)
        ));
    }

    #[test]
    fn a_plugin_reading_folders_loads_only_as_the_verified_build() {
        let text = reading_folders(MANIFEST);
        let manifest = Manifest::parse(&text).expect("valid manifest");
        let mut verified = Verified::new();
        assert!(may_load(&verified, &manifest, &text, b"wasm").is_err());
        verified.insert("bandcamp".into(), VerifiedBuild::of(&text, b"wasm"));
        assert!(may_load(&verified, &manifest, &text, b"wasm").is_ok());
        assert!(may_load(&verified, &manifest, &text, b"tampered").is_err());
        let edited = text.replace("Bandcamp", "Bandcamp!");
        assert!(may_load(&verified, &manifest, &edited, b"wasm").is_err());

        let plain = Manifest::parse(MANIFEST).expect("valid manifest");
        assert!(may_load(&Verified::new(), &plain, MANIFEST, b"anything").is_ok());
    }

    #[test]
    fn the_verified_record_is_written_and_forgotten() {
        let dir = std::env::temp_dir().join(format!("kopuz-verified-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(read_verified(&dir).is_empty());
        let build = VerifiedBuild::of("manifest", b"wasm");
        set_verified(&dir, "bandcamp", Some(build.clone())).unwrap();
        set_verified(&dir, "other", Some(VerifiedBuild::of("m", b"w"))).unwrap();
        assert_eq!(read_verified(&dir).get("bandcamp"), Some(&build));
        set_verified(&dir, "bandcamp", None).unwrap();
        let left = read_verified(&dir);
        assert!(!left.contains_key("bandcamp"));
        assert!(left.contains_key("other"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn installing_replaces_the_old_version_and_uninstalling_removes_it() {
        let dir = std::env::temp_dir().join(format!("kopuz-market-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let installed = install(&dir, "bandcamp", "v1", b"one").unwrap();
        assert_eq!(
            std::fs::read(installed.join(COMPONENT_FILE)).unwrap(),
            b"one"
        );
        install(&dir, "bandcamp", "v2", b"two").unwrap();
        assert_eq!(
            std::fs::read_to_string(installed.join(MANIFEST_FILE)).unwrap(),
            "v2"
        );
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name())
            .collect();
        assert_eq!(
            leftovers,
            ["bandcamp"],
            "no staging directory is left behind"
        );

        uninstall(&dir, &dir.join("bandcamp")).unwrap();
        assert!(!installed.exists());
        uninstall(&dir, &dir.join("bandcamp")).expect("removing what is gone is not an error");
        assert!(uninstall(&dir, &dir.join("a").join("b")).is_err());
        assert!(uninstall(&dir, dir.parent().unwrap()).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_id_cannot_reach_outside_the_plugin_directory() {
        let dir = Path::new("/plugins");
        for id in ["../etc", "a/b", "", ".hidden", "-flag", "UPPER"] {
            assert!(plugin_dir(dir, id).is_err(), "{id}");
        }
        assert_eq!(
            plugin_dir(dir, "band-camp_2").unwrap(),
            dir.join("band-camp_2")
        );
    }
}
