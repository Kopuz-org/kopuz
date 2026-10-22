//! The community marketplace: a JSON index naming each plugin's manifest and
//! component and the SHA-256 of both. Nothing is trusted beyond those hashes,
//! and the manifest is checked again before anything is written. A listing
//! may not claim an official plugin's id or a built-in service.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::manifest::{Manifest, ManifestError};
use crate::registry::{COMPONENT_FILE, MANIFEST_FILE};

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
    #[error("the plugin's manifest asks for more than the marketplace lists")]
    PermissionsDiffer,
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

pub fn verify(download: &Download, bytes: &[u8]) -> Result<(), MarketError> {
    let digest = Sha256::digest(bytes);
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    if hex.eq_ignore_ascii_case(download.sha256.trim()) {
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
    if declared != asked || services != listed_services {
        return Err(MarketError::PermissionsDiffer);
    }
    Ok(manifest)
}

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

pub fn uninstall(dir: &Path, id: &str) -> Result<(), MarketError> {
    let target = plugin_dir(dir, id)?;
    match std::fs::remove_dir_all(&target) {
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

    fn sha(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
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

        uninstall(&dir, "bandcamp").unwrap();
        assert!(!installed.exists());
        uninstall(&dir, "bandcamp").expect("removing what is gone is not an error");
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
