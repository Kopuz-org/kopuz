//! A plugin swapped under an open session: the call already running finishes
//! on the code it started on, and the next call runs the new code.

use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use plugin_host::{Code, Origin, Plugin, Session, manifest::Manifest, wit};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Notify, Semaphore};

/// The bundled Subsonic plugin, with its allowlist replaced by `network`.
fn subsonic(network: &str) -> Arc<Plugin> {
    let bundled = bundled_plugins::all()
        .iter()
        .find(|bundled| bundled.manifest.contains("id = \"subsonic\""))
        .expect("the subsonic plugin is bundled");
    let text = bundled
        .manifest
        .replace("network = [\"$server\"]", &format!("network = {network}"));
    let manifest = Manifest::parse(&text).expect("manifest");
    let wasm = bundled.wasm.expect("the subsonic plugin is a component");
    Arc::new(Plugin::new(
        manifest,
        Code::Wasm(Cow::Borrowed(wasm)),
        Origin::Bundled,
    ))
}

/// Answers a Subsonic ping once `release` is signalled, counting requests.
async fn held_server(arrived: Arc<Notify>, release: Arc<Semaphore>) -> (String, Arc<AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind");
    let url = format!("http://{}", listener.local_addr().expect("address"));
    let seen = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&seen);
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let arrived = Arc::clone(&arrived);
            let release = Arc::clone(&release);
            let count = Arc::clone(&count);
            tokio::spawn(async move {
                let mut buffer = vec![0u8; 16 * 1024];
                let _ = stream.read(&mut buffer).await;
                count.fetch_add(1, Ordering::SeqCst);
                arrived.notify_one();
                let _permit = release.acquire().await;
                let body = r#"{"subsonic-response":{"status":"ok","version":"1.16.1"}}"#;
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes()).await;
            });
        }
    });
    (url, seen)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_running_during_a_swap_finishes_on_the_old_plugin() {
    let arrived = Arc::new(Notify::new());
    let release = Arc::new(Semaphore::new(0));
    let (url, requests) = held_server(Arc::clone(&arrived), Arc::clone(&release)).await;

    let old = subsonic("[\"$server\"]");
    let session = Arc::new(Session::new(
        Arc::clone(&old),
        wit::Connection {
            service: "subsonic".into(),
            url,
            token: "hunter2".into(),
            user_id: "someone".into(),
            device_id: String::new(),
            account: None,
            folders: Vec::new(),
        },
    ));

    let running = tokio::spawn({
        let session = Arc::clone(&session);
        async move { session.validate().await }
    });
    tokio::time::timeout(std::time::Duration::from_secs(60), arrived.notified())
        .await
        .expect("the first call reached the server");

    // The new build may reach nothing, so a call that runs it never gets out.
    let new = subsonic("[]");
    session.swap(Arc::clone(&new));
    assert!(Arc::ptr_eq(&session.plugin(), &new));
    let after = session.validate().await.expect("the new plugin runs");
    assert_eq!(after, wit::AuthOutcome::Unreachable);

    release.add_permits(1);
    let before = running.await.expect("join").expect("the old plugin runs");
    assert_eq!(before, wit::AuthOutcome::Valid);
    assert_eq!(requests.load(Ordering::SeqCst), 1);

    // The old instance was not pooled again, so the old code is gone for good.
    let again = session.validate().await.expect("the new plugin runs");
    assert_eq!(again, wit::AuthOutcome::Unreachable);
    assert_eq!(requests.load(Ordering::SeqCst), 1);
}
