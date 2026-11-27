//! An integration plugin set up end to end: the webhook example, installed in
//! the plugin directory, published under Integrations with its own fields,
//! checked against a local fake server once it is turned on, and kept running.

use std::sync::{Arc, Mutex};

use api::{ErrorCode, FieldKind, FieldValue, IntegrationInfo, Text};
use daemon::{ConfigService, CredentialStore, IntegrationService, PresencePause};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const ID: &str = "plugin:webhook";

/// Answers each request with the next status in line, then 200, and keeps the requests.
struct Hook {
    url: String,
    received: Arc<Mutex<Vec<String>>>,
    statuses: Arc<Mutex<Vec<u16>>>,
}

impl Hook {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = listener.local_addr().expect("address");
        let received = Arc::new(Mutex::new(Vec::new()));
        let statuses: Arc<Mutex<Vec<u16>>> = Arc::new(Mutex::new(Vec::new()));
        let (kept, queued) = (Arc::clone(&received), Arc::clone(&statuses));
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut buffer = vec![0u8; 16 * 1024];
                let read = stream.read(&mut buffer).await.unwrap_or(0);
                kept.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&buffer[..read]).into_owned());
                let status = {
                    let mut queued = queued.lock().unwrap();
                    if queued.is_empty() {
                        200
                    } else {
                        queued.remove(0)
                    }
                };
                let reply = format!(
                    "HTTP/1.1 {status} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
                let _ = stream.write_all(reply.as_bytes()).await;
            }
        });
        Self {
            url: format!("http://{address}/hook"),
            received,
            statuses,
        }
    }

    fn refuse_next(&self) {
        self.statuses.lock().unwrap().push(500);
    }

    fn requests(&self) -> usize {
        self.received.lock().unwrap().len()
    }
}

/// The webhook example's files, written where an installed plugin goes.
fn install_webhook(dir: &std::path::Path) {
    install_example(dir, "webhook");
}

/// An example plugin's files, written where an installed plugin goes.
fn install_example(dir: &std::path::Path, id: &str) {
    install_example_as(dir, id, |manifest| manifest.to_string());
}

/// Install the example `id` with its manifest passed through `edit`.
fn install_example_as(dir: &std::path::Path, id: &str, edit: impl Fn(&str) -> String) {
    let example = bundled_plugins::examples()
        .iter()
        .find(|example| example.manifest.contains(&format!("id = \"{id}\"")))
        .expect("the examples are built with the examples feature");
    let at = dir.join(id);
    std::fs::create_dir_all(&at).unwrap();
    std::fs::write(at.join(plugin_host::MANIFEST_FILE), edit(example.manifest)).unwrap();
    std::fs::write(
        at.join(plugin_host::COMPONENT_FILE),
        example.wasm.expect("a component"),
    )
    .unwrap();
}

fn answer(key: &str, value: &str) -> FieldValue {
    FieldValue::new(key, value)
}

fn webhook(listed: &[IntegrationInfo]) -> &IntegrationInfo {
    listed
        .iter()
        .find(|info| info.id == ID)
        .expect("the webhook is listed")
}

/// One test, because the plugin directory is per process.
#[tokio::test(flavor = "multi_thread")]
async fn an_integration_plugin_is_set_up_from_its_own_fields() {
    let dir = tempfile::tempdir().expect("tempdir");
    let plugins = dir.path().join("plugins");
    install_webhook(&plugins);
    // Its panel narrowed to albums, so a page it does not name can be refused.
    install_example_as(&plugins, "remote", |manifest| {
        manifest.replace(
            "places = [\"album\", \"artist\", \"now-playing\"]",
            "places = [\"album\"]",
        )
    });
    server::plugins::init(Some(plugins), Vec::new()).await;

    let database = db::init(&dir.path().join("kopuz.db")).await.expect("db");
    let config = Arc::new(ConfigService::new(
        database.clone(),
        dir.path().join("settings.toml"),
        config::Settings::default(),
    ));
    let credentials = CredentialStore::load(&database).await.expect("credentials");
    let pause = PresencePause::load(&database).await.expect("pause");
    let service = IntegrationService::new(config, credentials, pause);

    let listed = service.list().await;
    let info = webhook(&listed);
    assert_eq!(info.name, Text::Literal("Webhook".into()));
    assert!(!info.configured);
    let keys: Vec<&str> = info.fields.iter().map(|field| field.key.as_str()).collect();
    assert_eq!(
        keys[..3],
        ["$enabled", "url", "token"],
        "the toggle, then the manifest's own"
    );
    assert_eq!(info.fields[0].label, Text::Literal("Webhook".into()));
    assert_eq!(info.fields[2].kind, FieldKind::Secret);

    let hook = Hook::start().await;
    // Answered but still off: nothing is checked, nothing runs.
    service
        .set_settings(
            ID,
            vec![answer("url", &hook.url), answer("token", "hunter2")],
        )
        .await
        .expect("answered");
    assert_eq!(hook.requests(), 0);
    assert!(service.plugins().running().is_empty());

    // Turning it on checks with the plugin, which the address refuses.
    hook.refuse_next();
    let refused = service
        .set_settings(ID, vec![answer("$enabled", "true")])
        .await
        .expect_err("the check failed");
    assert_eq!(refused.code, ErrorCode::InvalidInput);
    assert!(refused.message.contains("500"), "{refused:?}");
    assert!(
        !webhook(&service.list().await).configured,
        "a refused answer is not kept"
    );
    assert!(service.plugins().running().is_empty());

    let on = service
        .set_settings(ID, vec![answer("$enabled", "true")])
        .await
        .expect("accepted");
    assert!(on.configured);
    assert_eq!(hook.requests(), 2);
    let token = on
        .fields
        .iter()
        .find(|field| field.key == "token")
        .expect("token");
    assert_eq!(token.value, None, "a secret is never sent back out");
    let running = service.plugins().running();
    assert_eq!(running.len(), 1);
    assert_eq!(running[0].plugin().id(), "webhook");

    // Saving what it already runs with neither asks the plugin again nor reopens it.
    assert!(Arc::ptr_eq(&running[0], &service.plugins().running()[0]));
    service
        .set_settings(ID, vec![answer("token", "")])
        .await
        .expect("an empty secret keeps the stored one");
    service
        .set_settings(ID, vec![answer("url", &hook.url)])
        .await
        .expect("the same address again");
    assert_eq!(
        hook.requests(),
        2,
        "nothing changed, so nothing was checked"
    );
    assert!(Arc::ptr_eq(&running[0], &service.plugins().running()[0]));

    // A new answer closes the running copy before the check, and the copy
    // that checked is the one that runs: one connection at a time.
    let moved = Hook::start().await;
    service
        .set_settings(ID, vec![answer("url", &moved.url)])
        .await
        .expect("a new address");
    assert!(running[0].is_closed(), "the old copy closed first");
    let now = service.plugins().running();
    assert_eq!(now.len(), 1);
    assert!(!now[0].is_closed());
    assert_eq!(
        moved.requests(),
        1,
        "checked once, by the copy that now runs"
    );
    assert!(
        now[0]
            .settings()
            .iter()
            .any(|(key, value)| key == "url" && *value == moved.url)
    );

    service.clear(ID).await.expect("cleared");
    assert!(!webhook(&service.list().await).configured);
    assert!(service.plugins().running().is_empty());
    assert!(
        database
            .integration_credentials()
            .await
            .expect("rows")
            .keys()
            .all(|key| !key.starts_with("plugin/webhook/")),
        "clearing drops every answer"
    );

    // An uninstalled plugin is forgotten by id, secrets and all, loaded or not.
    service
        .set_settings(ID, vec![answer("token", "hunter2")])
        .await
        .expect("answered while off");
    service
        .plugins()
        .forget("webhook")
        .await
        .expect("forgotten");
    assert!(
        database
            .integration_credentials()
            .await
            .expect("rows")
            .keys()
            .all(|key| !key.starts_with("plugin/webhook/")),
        "forgetting leaves no secret behind"
    );

    // Only a plugin that is turned on offers its actions, in its manifest's order.
    assert!(service.plugins().actions().is_empty());
    let orders = Hook::start().await;
    service
        .set_settings(
            "plugin:remote",
            vec![answer("url", &orders.url), answer("$enabled", "true")],
        )
        .await
        .expect("the remote is on");
    let actions = service.plugins().actions();
    let ids: Vec<&str> = actions.iter().map(|action| action.id.as_str()).collect();
    assert_eq!(ids, ["find", "more-by-artist"]);
    assert_eq!(actions[0].places, [api::ActionPlace::Palette]);
    assert!(actions[0].ask.is_some());

    let elsewhere = service
        .plugins()
        .run_action("remote", "more-by-artist", api::ActionTarget::None, None)
        .await
        .expect_err("a track action run from the palette");
    assert_eq!(elsewhere.code, ErrorCode::InvalidInput);
    let off = service
        .plugins()
        .run_action("webhook", "find", api::ActionTarget::None, None)
        .await
        .expect_err("the webhook is off and has no actions");
    assert_eq!(off.code, ErrorCode::NotFound);
    // No bridge in this test, so the plugin's library search is refused and it says so.
    let refused = service
        .plugins()
        .run_action(
            "remote",
            "find",
            api::ActionTarget::None,
            Some("blue".into()),
        )
        .await
        .expect_err("nothing to search");
    assert!(refused.message.contains("library"), "{refused:?}");

    // Panels are listed like actions, and drawn only where the manifest puts them.
    let panels = service.plugins().panels();
    assert_eq!(panels.len(), 1);
    assert_eq!(panels[0].id, "more-by-artist");
    assert_eq!(panels[0].places, [api::PanelPlace::Album]);
    let not_here = service
        .plugins()
        .render_panel(
            "remote",
            "more-by-artist",
            api::PanelTarget::Artist("key".into()),
        )
        .await
        .expect_err("the panel is only drawn on albums");
    assert_eq!(not_here.code, ErrorCode::InvalidInput);
    assert!(not_here.message.contains("not drawn there"), "{not_here:?}");
    let undrawn = service
        .plugins()
        .render_panel("remote", "nothing", api::PanelTarget::Album("al".into()))
        .await
        .expect_err("no such panel");
    assert_eq!(undrawn.code, ErrorCode::NotFound);
    let unread = service
        .plugins()
        .render_panel(
            "remote",
            "more-by-artist",
            api::PanelTarget::Album("al".into()),
        )
        .await
        .expect_err("no bridge, so the library read is refused");
    assert!(unread.message.contains("library"), "{unread:?}");

    // A plugin that provides transitions plans one between albums, checked whole.
    let song = |key: &str, album: &str| plugin_host::wit::Song {
        key: key.into(),
        title: key.into(),
        artist: "Ada".into(),
        album: album.into(),
        duration_ms: Some(200_000),
    };
    let plan = service
        .plugins()
        .transition(&song("a", "First"), &song("b", "Second"))
        .await
        .expect("the remote plans one");
    assert_eq!(plan.length, 8.0);
    assert!(
        service
            .plugins()
            .transition(&song("a", "First"), &song("c", "First"))
            .await
            .is_none(),
        "within an album it leaves the crossfade"
    );
}
