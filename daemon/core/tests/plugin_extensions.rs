//! A plugin a Flatpak extension provides: it loads beneath the plugin
//! directory, a marketplace install of its id replaces it, and the daemon
//! never removes its read-only files.

use std::path::{Path, PathBuf};

use api::ErrorCode;
use daemon::PluginService;
use sha2::{Digest, Sha256};

const MANIFEST: &str = r#"
id = "echo"
name = "Echo"
version = "1.0.0"
api = "0.1"

[permissions]
network = ["$server"]

[[service]]
id = "echo-service"
name = "Echo"
sign-in = "password"
"#;

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
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
    let dir = std::env::temp_dir().join(format!("kopuz-extension-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// A `file://` marketplace in `dir` listing echo 1.0.0.
fn marketplace(dir: &Path) -> String {
    std::fs::create_dir_all(dir).unwrap();
    let manifest_path = dir.join("echo.toml");
    let component_path = dir.join("echo.wasm");
    std::fs::write(&manifest_path, MANIFEST).unwrap();
    std::fs::write(&component_path, subsonic_component()).unwrap();
    let index = serde_json::json!({
        "schema": 1,
        "plugins": [{
            "id": "echo",
            "name": "Echo",
            "version": "1.0.0",
            "api": "0.1",
            "services": ["echo-service"],
            "network": ["$server"],
            "manifest": { "url": format!("file://{}", manifest_path.display()), "sha256": sha(MANIFEST.as_bytes()) },
            "component": { "url": format!("file://{}", component_path.display()), "sha256": sha(subsonic_component()) },
        }],
    });
    let index_path = dir.join("index.json");
    std::fs::write(&index_path, index.to_string()).unwrap();
    format!("file://{}", index_path.display())
}

fn serving_origin() -> Option<plugin_host::Origin> {
    server::plugins::registry()
        .for_service("echo-service")
        .map(|plugin| plugin.origin())
}

/// One test, because the plugin directory is per process.
#[tokio::test(flavor = "multi_thread")]
async fn an_extension_plugin_loads_yields_to_an_install_and_is_never_removed() {
    let plugins = temp("installed");
    let extensions = temp("mounted");
    let mounted = extensions.join("echo");
    std::fs::create_dir_all(&mounted).unwrap();
    std::fs::write(
        mounted.join(plugin_host::MANIFEST_FILE),
        MANIFEST.replace("1.0.0", "0.9.0"),
    )
    .unwrap();
    std::fs::write(
        mounted.join(plugin_host::COMPONENT_FILE),
        subsonic_component(),
    )
    .unwrap();
    server::plugins::init(Some(plugins.clone()), Some(extensions.clone())).await;
    assert_eq!(serving_origin(), Some(plugin_host::Origin::Extension));

    let market = PluginService::new(marketplace(&temp("market")));
    let refused = market
        .uninstall("echo")
        .await
        .expect_err("an extension's files are Flatpak's");
    assert_eq!(refused.code, ErrorCode::InvalidInput);
    assert!(refused.message.contains("Flatpak"), "{}", refused.message);
    assert!(mounted.join(plugin_host::COMPONENT_FILE).is_file());

    let installed = market
        .install("echo", &["$server".into()])
        .await
        .expect("a marketplace install may take an extension's id");
    assert_eq!(installed.installed_version.as_deref(), Some("1.0.0"));
    assert_eq!(serving_origin(), Some(plugin_host::Origin::Installed));
    assert!(mounted.join(plugin_host::COMPONENT_FILE).is_file());

    market
        .uninstall("echo")
        .await
        .expect("the installed copy is removed");
    assert!(!plugins.join("echo").exists());
    assert_eq!(serving_origin(), Some(plugin_host::Origin::Extension));
    assert!(mounted.join(plugin_host::COMPONENT_FILE).is_file());

    for dir in [plugins, extensions, temp("market")] {
        std::fs::remove_dir_all(dir).ok();
    }
}
