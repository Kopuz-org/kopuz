//! The Lua runtime end to end, against a plugin that really sits on disk.
//!
//! One test, deliberately: `KOPUZ_PLUGIN_PATH` and the registry are both
//! process-wide, so a second test in this binary would race this one on the
//! environment and on the loaded instance.

use std::path::PathBuf;

use server::plugin::{self, PluginError, registry};
use server::source::FavoritesSync;

const ID: &str = "luafixture";

const MANIFEST: &str = r##"id = "luafixture"
name = "Lua Fixture"
version = "9.9"
api = 1
icon = "fa-solid fa-flask"
accent = "#123456"
"##;

/// A manifest this build cannot run. Discovery must drop it and keep going.
const FUTURE_MANIFEST: &str = r#"id = "broken"
name = "From The Future"
version = "1.0"
api = 99
"#;

/// Exports one of each shape the host cares about: a handshake, a call that
/// hands back a table, a classified failure, and a store round trip. What it
/// does *not* export is the point of the `Unsupported` case below.
const ENTRY: &str = r#"
local M = {}

function M.setup(ctx)
  return {
    name = "Lua Fixture",
    version = "9.9",
    capabilities = { sync = true, discover = true, favorites_sync = "paginated" },
    auth_required = false,
    web_url_template = "https://example.test/{id}",
  }
end

function M.fetch_favorites()
  return { "one", "two", "three" }
end

function M.validate()
  kopuz.fail("auth", "the fixture is never signed in")
end

function M.remember(value)
  kopuz.store.set("token", value)
end

function M.recall()
  return kopuz.store.get("token")
end

return M
"#;

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn install() -> Self {
        let root = std::env::temp_dir().join(format!("kopuz-lua-{}", uuid::Uuid::new_v4()));
        write_plugin(&root, ID, MANIFEST);
        write_plugin(&root, "broken", FUTURE_MANIFEST);
        Self { root }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
        // The plugin's data directory resolves against the real config directory
        // and takes no override, so what `kopuz.store` wrote sits outside the
        // temp root and has to be swept separately.
        let _ = std::fs::remove_dir_all(plugin::manifest::data_dir_for(ID));
    }
}

fn write_plugin(root: &std::path::Path, dir_name: &str, manifest: &str) {
    let dir = root.join(dir_name);
    std::fs::create_dir_all(&dir).expect("create the plugin directory");
    std::fs::write(dir.join("plugin.toml"), manifest).expect("write the manifest");
    std::fs::write(dir.join("init.lua"), ENTRY).expect("write the entry chunk");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_runtime_drives_a_lua_plugin_on_disk() {
    let fixture = Fixture::install();
    // SAFETY: nothing has touched the registry yet, and this is the only test in
    // the binary, so no other thread is reading the environment.
    unsafe { std::env::set_var("KOPUZ_PLUGIN_PATH", &fixture.root) };

    let registry = registry();
    registry.rescan();

    // The user's own plugins directory stays in the search path, so every
    // assertion here is by id and never by count.
    let manifests = registry.manifests();
    let Some(found) = manifests.iter().find(|m| m.id == ID) else {
        panic!("the fixture must be discovered through KOPUZ_PLUGIN_PATH");
    };
    assert_eq!(found.api, 1);
    assert_eq!(found.icon.as_deref(), Some("fa-solid fa-flask"));
    assert!(
        !manifests.iter().any(|m| m.id == "broken"),
        "a manifest claiming an api generation this build cannot run must be skipped"
    );

    let instance = registry.instance(ID).await.expect("the fixture must load");
    assert!(registry.loaded(ID));

    // Capabilities come from setup()'s handshake, not from the manifest.
    let caps = registry
        .cached_capabilities(ID)
        .expect("the handshake must reach the registry");
    assert!(caps.sync && caps.discover);
    assert_eq!(caps.favorites_sync, FavoritesSync::Paginated);
    assert_eq!(
        registry.cached_web_url_template(ID).as_deref(),
        Some("https://example.test/{id}")
    );

    let favorites: Vec<String> = instance
        .call("fetch_favorites", ())
        .await
        .expect("a returned table must round-trip");
    assert_eq!(favorites, ["one", "two", "three"]);

    // An absent export is how a plugin declines an optional operation.
    let err = instance
        .call_unit("discover_home", ())
        .await
        .expect_err("an unexported function cannot succeed");
    assert!(
        matches!(&err, PluginError::Unsupported(op) if op == "discover_home"),
        "an absent export must be Unsupported, got {err:?}"
    );

    assert_eq!(
        instance.call_unit("validate", ()).await,
        Err(PluginError::Auth),
        "kopuz.fail(\"auth\", …) must survive as an auth failure"
    );

    instance
        .call_unit("remember", "tok-42".to_string())
        .await
        .expect("the store must accept a write");
    let recalled: String = instance
        .call("recall", ())
        .await
        .expect("the store must answer within the same run");
    assert_eq!(recalled, "tok-42");

    registry.disconnect(ID).await;
    assert!(!registry.loaded(ID));

    let Err(missing) = registry.instance("definitely-not-installed").await else {
        panic!("an id with no manifest must not load");
    };
    assert!(matches!(missing, PluginError::NotInstalled(_)));

    plugin::shutdown_all().await;
}
