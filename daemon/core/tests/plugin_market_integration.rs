//! An integration plugin installed from a marketplace, end to end: listed
//! with no services, verified and installed like a source plugin, then offered
//! under Connectivity, and gone from there once it is removed.

use std::sync::Arc;

use daemon::{ConfigService, CredentialStore, IntegrationService, PluginService, PresencePause};
use sha2::{Digest, Sha256};

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn webhook() -> &'static plugin_host::Bundled {
    bundled_plugins::examples()
        .iter()
        .find(|example| example.manifest.contains("id = \"webhook\""))
        .expect("the webhook example is built with the examples feature")
}

/// A `file://` index in `dir` listing the webhook example.
fn marketplace(dir: &std::path::Path) -> String {
    let example = webhook();
    let wasm = example.wasm.expect("a component");
    std::fs::create_dir_all(dir).unwrap();
    let manifest_path = dir.join("webhook.toml");
    let component_path = dir.join("webhook.wasm");
    std::fs::write(&manifest_path, example.manifest).unwrap();
    std::fs::write(&component_path, wasm).unwrap();
    let index = serde_json::json!({
        "schema": 1,
        "plugins": [{
            "id": "webhook",
            "name": "Webhook",
            "version": "0.1.0",
            "api": "0.2",
            "author": "tests",
            "services": [],
            "network": ["$server"],
            "integration": true,
            "manifest": { "url": format!("file://{}", manifest_path.display()), "sha256": sha(example.manifest.as_bytes()) },
            "component": { "url": format!("file://{}", component_path.display()), "sha256": sha(wasm) },
        }],
    });
    let index_path = dir.join("index.json");
    std::fs::write(&index_path, index.to_string()).unwrap();
    format!("file://{}", index_path.display())
}

/// One test, because the plugin directory is per process.
#[tokio::test(flavor = "multi_thread")]
async fn an_integration_plugin_installs_from_the_marketplace() {
    let dir = tempfile::tempdir().expect("tempdir");
    let plugins = dir.path().join("plugins");
    server::plugins::init(Some(plugins.clone()), Vec::new()).await;

    let database = db::init(&dir.path().join("kopuz.db")).await.expect("db");
    let config = Arc::new(ConfigService::new(
        database.clone(),
        dir.path().join("settings.toml"),
        config::Settings::default(),
    ));
    let credentials = CredentialStore::load(&database).await.expect("credentials");
    let pause = PresencePause::load(&database).await.expect("pause");
    let integrations = IntegrationService::new(config, credentials, pause);
    let offered =
        |listed: &[api::IntegrationInfo]| listed.iter().any(|info| info.id == "plugin:webhook");
    assert!(!offered(&integrations.list().await));

    let market = PluginService::new(marketplace(&dir.path().join("market")));
    let listed = market.catalog().await;
    let listing = listed
        .plugins
        .iter()
        .find(|plugin| plugin.id == "webhook")
        .expect("listed");
    assert!(listing.services.is_empty());
    assert!(listing.integration, "marked before it is installed");

    let installed = market
        .install("webhook", &["$server".to_string()])
        .await
        .expect("installed");
    assert_eq!(installed.installed_version.as_deref(), Some("0.1.0"));
    assert!(installed.integration);
    assert!(plugins.join("webhook").join("plugin.wasm").is_file());
    assert!(
        offered(&integrations.list().await),
        "offered once installed"
    );

    market.uninstall("webhook").await.expect("removed");
    assert!(!plugins.join("webhook").exists());
    assert!(!offered(&integrations.list().await), "gone once removed");
}
