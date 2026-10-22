//! Installing a community plugin from a marketplace, end to end, against a
//! local `file://` index: the download is verified, the plugin loads, the
//! service it adds is offered, and removing it takes the service away.
//!
//! The component is the official Subsonic one under a community manifest,
//! which is a real component this build can run without building another.

use std::path::{Path, PathBuf};

use api::ErrorCode;
use daemon::PluginService;
use sha2::{Digest, Sha256};

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

const MANIFEST: &str = r#"
id = "echo"
name = "Echo"
version = "1.0.0"
api = "0.1"
description = "A test plugin."

[permissions]
network = ["$server"]

[[service]]
id = "echo-service"
name = "Echo"
summary = "Plays back what it hears"
sign-in = "password"
capabilities = { sync = true }
"#;

/// A marketplace in `dir`: one listing whose files are `manifest` and
/// `component`, with `listed_hash` standing in for the component's hash.
fn marketplace(
    dir: &Path,
    id: &str,
    manifest: &str,
    component: &[u8],
    listed_hash: Option<&str>,
) -> String {
    std::fs::create_dir_all(dir).unwrap();
    let manifest_path = dir.join(format!("{id}.toml"));
    let component_path = dir.join(format!("{id}.wasm"));
    std::fs::write(&manifest_path, manifest).unwrap();
    std::fs::write(&component_path, component).unwrap();
    let index = serde_json::json!({
        "schema": 1,
        "plugins": [{
            "id": id,
            "name": "Echo",
            "version": "1.0.0",
            "api": "0.1",
            "author": "tests",
            "services": ["echo-service"],
            "network": ["$server"],
            "manifest": { "url": format!("file://{}", manifest_path.display()), "sha256": sha(manifest.as_bytes()) },
            "component": {
                "url": format!("file://{}", component_path.display()),
                "sha256": listed_hash.map_or_else(|| sha(component), str::to_string),
            },
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
    let dir = std::env::temp_dir().join(format!("kopuz-market-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// One test, because the plugin directory is per process.
#[tokio::test(flavor = "multi_thread")]
async fn a_community_plugin_installs_offers_its_service_and_uninstalls() {
    let plugins = temp("installed");
    server::plugins::init(Some(plugins.clone()));

    // A tampered component is refused before anything is written.
    let tampered = PluginService::new(marketplace(
        &temp("tampered"),
        "echo",
        MANIFEST,
        subsonic_component(),
        Some(&"0".repeat(64)),
    ));
    let refused = tampered.install("echo").await.expect_err("hash mismatch");
    assert_eq!(refused.code, ErrorCode::InvalidInput);
    assert!(!plugins.join("echo").exists());

    // An official plugin is not the marketplace's to replace.
    let impostor = PluginService::new(marketplace(
        &temp("impostor"),
        "subsonic",
        &MANIFEST.replace("id = \"echo\"", "id = \"subsonic\""),
        subsonic_component(),
        None,
    ));
    assert!(impostor.install("subsonic").await.is_err());

    // The real thing.
    let market = PluginService::new(marketplace(
        &temp("market"),
        "echo",
        MANIFEST,
        subsonic_component(),
        None,
    ));
    let before = market.catalog().await;
    let listed = before
        .plugins
        .iter()
        .find(|plugin| plugin.id == "echo")
        .expect("listed");
    assert_eq!(listed.installed_version, None);
    assert_eq!(listed.network, ["$server"]);
    assert!(
        before
            .plugins
            .iter()
            .all(|plugin| plugin.id != "echo" || plugin.origin == api::PluginOrigin::Community)
    );

    let installed = market.install("echo").await.expect("installed");
    assert_eq!(installed.installed_version.as_deref(), Some("1.0.0"));
    assert_eq!(installed.origin, api::PluginOrigin::Community);
    assert!(plugins.join("echo").join("plugin.wasm").is_file());

    let offered = daemon::services::all();
    let echo = offered
        .iter()
        .find(|service| service.id == "echo-service")
        .expect("the plugin's service is offered");
    assert!(echo.experimental, "a community service is marked as such");
    assert_eq!(echo.fields.len(), 1, "it asks for its server's address");
    assert_eq!(
        daemon::services::resolve("echo-service"),
        Some(config::MusicService::Plugin)
    );

    // Official plugins cannot be removed; community ones can.
    assert!(market.uninstall("subsonic").await.is_err());
    market.uninstall("echo").await.expect("removed");
    assert!(!plugins.join("echo").exists());
    assert!(
        daemon::services::all()
            .iter()
            .all(|service| service.id != "echo-service")
    );
    assert_eq!(daemon::services::resolve("echo-service"), None);

    std::fs::remove_dir_all(&plugins).ok();
}
