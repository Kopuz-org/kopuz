//! End-to-end lifecycle against a real child process.
//!
//! The fixture is a shell script rather than the `plugin-example` binary so the
//! test does not depend on another crate's build artifacts being present, and
//! so it can be made to crash on demand — the case that matters most, because a
//! plugin host that hangs on a dead child is worse than no plugin host.

use std::path::{Path, PathBuf};

use server::plugin::manifest::{PluginManifest, discover_in};
use server::plugin::wire::{self, PROTOCOL_VERSION};
use server::plugin::{PluginClient, PluginEvent};
use server::source::SourceError;

/// A plugin that answers `initialize`, `ping` and `echo`, and exits on
/// `explode` — a crash the host must surface rather than wait out.
const FIXTURE: &str = r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  case "$line" in
    *'"initialize"'*)
      printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"protocol\":PROTO,\"name\":\"Fixture\",\"version\":\"9.9\",\"capabilities\":{\"sync\":true,\"discover\":true,\"favorites_sync\":\"Paginated\"},\"auth_required\":false,\"data_base_url\":\"http://127.0.0.1:1\",\"data_token\":\"tok\",\"web_url_template\":\"https://example.test/{id}\"}}"
      ;;
    *'"explode"'*) exit 7 ;;
    *'"nudge"'*)
      printf '%s\n' '{"jsonrpc":"2.0","method":"library_changed","params":{}}'
      printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":null}"
      ;;
    *'"boom"'*)
      printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$id,\"error\":{\"code\":-32000,\"message\":\"nope\",\"data\":{\"kind\":\"unsupported\"}}}"
      ;;
    *'"shutdown"'*) exit 0 ;;
    *) printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"ok\":true}}" ;;
  esac
done
"#;

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn install(protocol: u32) -> Self {
        let root = std::env::temp_dir().join(format!("kopuz-plugin-test-{}", uuid::Uuid::new_v4()));
        let dir = root.join("fixture");
        std::fs::create_dir_all(&dir).expect("create the plugin directory");

        let script = dir.join("run.sh");
        std::fs::write(&script, FIXTURE.replace("PROTO", &protocol.to_string()))
            .expect("write the fixture");
        make_executable(&script);

        std::fs::write(
            dir.join("plugin.toml"),
            "id = \"fixture\"\nname = \"Fixture\"\nversion = \"9.9\"\nprotocol = 1\n\
             executable = \"run.sh\"\nicon = \"fa-solid fa-flask\"\naccent = \"#123456\"\n",
        )
        .expect("write the manifest");

        Self { root }
    }

    fn manifest(&self) -> PluginManifest {
        let mut found = discover_in(&self.root);
        assert_eq!(found.len(), 1, "the fixture manifest must be discovered");
        found.remove(0)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path).expect("stat").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).expect("chmod");
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) {}

#[tokio::test]
#[cfg_attr(not(unix), ignore = "the fixture plugin is a shell script")]
async fn handshake_exposes_manifest_and_capabilities() {
    let fixture = Fixture::install(PROTOCOL_VERSION);
    let manifest = fixture.manifest();
    assert_eq!(manifest.icon.as_deref(), Some("fa-solid fa-flask"));
    assert_eq!(manifest.accent.as_deref(), Some("#123456"));

    let client = PluginClient::connect(manifest)
        .await
        .expect("the fixture must connect");

    assert_eq!(
        client.identity(),
        ("Fixture".to_string(), "9.9".to_string())
    );
    let caps = client.capabilities();
    assert!(caps.sync && caps.discover);
    assert_eq!(
        caps.favorites_sync,
        server::source::FavoritesSync::Paginated
    );
    assert!(!client.auth_required());
    assert_eq!(
        client.web_url("abc").as_deref(),
        Some("https://example.test/abc")
    );

    client.shutdown().await;
}

#[tokio::test]
#[cfg_attr(not(unix), ignore = "the fixture plugin is a shell script")]
async fn a_protocol_mismatch_is_refused() {
    let fixture = Fixture::install(PROTOCOL_VERSION + 1);
    // `PluginClient` is deliberately not `Debug` (it owns a child process), so
    // the error is matched rather than unwrapped.
    let Err(err) = PluginClient::connect(fixture.manifest()).await else {
        panic!("a mismatched protocol must not connect");
    };
    let message = err.to_string();
    assert!(
        message.contains("protocol") && message.contains(&PROTOCOL_VERSION.to_string()),
        "the error must name both protocol versions, got {message:?}"
    );
}

#[tokio::test]
#[cfg_attr(not(unix), ignore = "the fixture plugin is a shell script")]
async fn calls_round_trip_and_errors_keep_their_kind() {
    let fixture = Fixture::install(PROTOCOL_VERSION);
    let client = PluginClient::connect(fixture.manifest())
        .await
        .expect("connect");

    let value: serde_json::Value = client
        .call(wire::method::PING, serde_json::json!({}))
        .await
        .expect("ping");
    assert_eq!(value, serde_json::json!({ "ok": true }));

    let err = client
        .call::<_, serde_json::Value>("boom", serde_json::json!({}))
        .await
        .expect_err("the fixture reports this one unsupported");
    assert!(
        matches!(&err, SourceError::Unsupported(op) if op == "nope"),
        "the error kind must survive the wire, got {err:?}"
    );

    client.shutdown().await;
}

#[tokio::test]
#[cfg_attr(not(unix), ignore = "the fixture plugin is a shell script")]
async fn notifications_reach_subscribers() {
    let fixture = Fixture::install(PROTOCOL_VERSION);
    let client = PluginClient::connect(fixture.manifest())
        .await
        .expect("connect");
    let mut events = client.subscribe();

    let _: serde_json::Value = client
        .call("nudge", serde_json::json!({}))
        .await
        .expect("nudge");

    let event = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
        .await
        .expect("a notification must arrive")
        .expect("the channel must stay open");
    assert_eq!(event, PluginEvent::LibraryChanged);

    client.shutdown().await;
}

#[tokio::test]
#[cfg_attr(not(unix), ignore = "the fixture plugin is a shell script")]
async fn a_crash_fails_the_call_and_the_next_one_respawns() {
    let fixture = Fixture::install(PROTOCOL_VERSION);
    let client = PluginClient::connect(fixture.manifest())
        .await
        .expect("connect");
    let mut events = client.subscribe();

    // The child exits without replying. The call must fail, not hang.
    let err = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        client.call::<_, serde_json::Value>("explode", serde_json::json!({})),
    )
    .await
    .expect("a crashed plugin must not hang the caller")
    .expect_err("the call cannot succeed");
    assert!(
        matches!(err, SourceError::Backend(_)),
        "a crash is a backend failure, got {err:?}"
    );

    let exited = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
        .await
        .expect("an exit must be announced")
        .expect("the channel must stay open");
    assert!(matches!(exited, PluginEvent::Exited { .. }));

    // The next call stands a fresh child back up (after the backoff).
    let value: serde_json::Value = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        client.call(wire::method::PING, serde_json::json!({})),
    )
    .await
    .expect("the respawn must not hang")
    .expect("the respawned plugin answers");
    assert_eq!(value, serde_json::json!({ "ok": true }));

    client.shutdown().await;
}
