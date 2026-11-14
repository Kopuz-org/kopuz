//! A plugin that reads folders, installed from a `file://` marketplace: only a
//! listing marked verified installs, the build is recorded as verified, its
//! service asks for folders instead of an address, and removing it forgets
//! the record.
//!
//! The component is the official Subsonic one under a manifest asking for
//! folders; loading is what is tested, not what it reads.

use std::path::{Path, PathBuf};

use api::ErrorCode;
use api::schema::FieldKind;
use daemon::PluginService;
use sha2::{Digest, Sha256};

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

const MANIFEST: &str = r#"
id = "songs"
name = "Songs"
version = "1.0.0"
api = "0.1"
description = "Songs a game keeps on this computer."

[permissions]
network = []
folders = "read"

[[service]]
id = "songs-service"
name = "Songs"
capabilities = { sync = true }
"#;

fn marketplace(dir: &Path, component: &[u8], folders: &str, verified: bool) -> String {
    std::fs::create_dir_all(dir).unwrap();
    let manifest_path = dir.join("songs.toml");
    let component_path = dir.join("songs.wasm");
    std::fs::write(&manifest_path, MANIFEST).unwrap();
    std::fs::write(&component_path, component).unwrap();
    let index = serde_json::json!({
        "schema": 1,
        "plugins": [{
            "id": "songs",
            "name": "Songs",
            "version": "1.0.0",
            "api": "0.1",
            "author": "tests",
            "services": ["songs-service"],
            "network": [],
            "folders": folders,
            "verified": verified,
            "manifest": { "url": format!("file://{}", manifest_path.display()), "sha256": sha(MANIFEST.as_bytes()) },
            "component": { "url": format!("file://{}", component_path.display()), "sha256": sha(component) },
        }],
    });
    let index_path = dir.join("index.json");
    std::fs::write(&index_path, index.to_string()).unwrap();
    format!("file://{}", index_path.display())
}

fn subsonic_component() -> &'static [u8] {
    bundled_plugins::all()
        .iter()
        .find(|bundled| bundled.manifest.contains("id = \"subsonic\""))
        .expect("the Subsonic plugin is bundled")
        .wasm
        .expect("it is a component")
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kopuz-verified-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// One test, because the plugin directory is per process.
#[tokio::test(flavor = "multi_thread")]
async fn only_a_verified_listing_installs_a_plugin_that_reads_folders() {
    let plugins = temp("installed");
    server::plugins::init(Some(plugins.clone()), Vec::new()).await;

    // Listed without the folders it asks for, it is refused.
    let quiet = PluginService::new(marketplace(
        &temp("quiet"),
        subsonic_component(),
        "none",
        true,
    ));
    assert!(quiet.install("songs", &[]).await.is_err());

    // Listed honestly but not verified, it is refused too.
    let unverified = PluginService::new(marketplace(
        &temp("unverified"),
        subsonic_component(),
        "read",
        false,
    ));
    let listed = unverified.catalog().await;
    let shown = listed
        .plugins
        .iter()
        .find(|plugin| plugin.id == "songs")
        .expect("listed");
    assert!(shown.reads_folders && !shown.verified);
    let refused = unverified
        .install("songs", &[])
        .await
        .expect_err("not verified");
    assert_eq!(refused.code, ErrorCode::InvalidInput);
    assert!(!plugins.join("songs").exists());

    let verified = PluginService::new(marketplace(
        &temp("verified"),
        subsonic_component(),
        "read",
        true,
    ));
    let installed = verified.install("songs", &[]).await.expect("installed");
    assert!(installed.verified);
    assert!(installed.reads_folders);
    let record = plugin_host::market::read_verified(&plugins);
    assert_eq!(
        record.get("songs"),
        Some(&plugin_host::market::VerifiedBuild::of(
            MANIFEST,
            subsonic_component()
        ))
    );

    // Its sources are folders, not an address.
    let offered = daemon::services::all();
    let service = offered
        .iter()
        .find(|service| service.id == "songs-service")
        .expect("the plugin's service is offered");
    assert_eq!(service.fields.len(), 1);
    assert!(matches!(service.fields[0].kind, FieldKind::Directories));

    // Edited in place, the files are no longer the verified build, so a
    // rescan keeps the build that was loaded.
    std::fs::write(
        plugins.join("songs").join(plugin_host::MANIFEST_FILE),
        MANIFEST.replace("Songs a game", "Anything a game"),
    )
    .unwrap();
    server::plugins::rescan().await;
    let loaded = server::plugins::registry()
        .plugins()
        .iter()
        .find(|plugin| plugin.id() == "songs")
        .map(|plugin| plugin.manifest().description.clone());
    assert_eq!(
        loaded.as_deref(),
        Some("Songs a game keeps on this computer.")
    );

    verified.uninstall("songs").await.expect("removed");
    assert!(!plugins.join("songs").exists());
    assert!(!plugin_host::market::read_verified(&plugins).contains_key("songs"));

    std::fs::remove_dir_all(&plugins).ok();
}
