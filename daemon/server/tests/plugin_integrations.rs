//! Integration plugins, run end to end: an example component, loaded by the
//! plugin host from the answers to its fields, reaching a fake server on a
//! local port through the address the person typed.

use std::sync::{Arc, Mutex};

use plugin_host::manifest::Manifest;
use plugin_host::{Code, Integration, Origin, Plugin, wit};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Answers every request with `status` and keeps what it was sent.
struct Hook {
    url: String,
    received: Arc<Mutex<Vec<String>>>,
}

impl Hook {
    async fn start(status: u16) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = listener.local_addr().expect("address");
        let received = Arc::new(Mutex::new(Vec::new()));
        let kept = Arc::clone(&received);
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let request = read_request(&mut stream).await;
                kept.lock().unwrap().push(request);
                let reply = format!(
                    "HTTP/1.1 {status} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
                let _ = stream.write_all(reply.as_bytes()).await;
            }
        });
        Self {
            url: format!("http://{address}/hook"),
            received,
        }
    }

    fn received(&self) -> Vec<String> {
        self.received.lock().unwrap().clone()
    }
}

/// The head and the body its `content-length` names.
async fn read_request(stream: &mut tokio::net::TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 4096];
    loop {
        let read = stream.read(&mut buffer).await.unwrap_or(0);
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
        let text = String::from_utf8_lossy(&bytes);
        if let Some(end) = text.find("\r\n\r\n") {
            let length = text[..end]
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())?
                })
                .unwrap_or(0);
            if bytes.len() >= end + 4 + length {
                break;
            }
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn webhook() -> Arc<Plugin> {
    let bundled = bundled_plugins::examples()
        .iter()
        .find(|example| example.manifest.contains("id = \"webhook\""))
        .expect("the webhook example is built with the examples feature");
    let manifest = Manifest::parse(bundled.manifest).expect("its manifest");
    let wasm = bundled.wasm.expect("it is a component");
    Arc::new(Plugin::new(
        manifest,
        Code::Wasm(std::borrow::Cow::Borrowed(wasm)),
        Origin::Installed,
    ))
}

fn answers(url: &str, token: &str) -> Vec<(String, String)> {
    vec![
        ("url".to_string(), url.to_string()),
        ("token".to_string(), token.to_string()),
    ]
}

#[tokio::test(flavor = "multi_thread")]
async fn an_integration_opens_with_its_answers_and_reaches_the_address_typed() {
    let hook = Hook::start(200).await;
    let integration = Integration::new(webhook(), answers(&hook.url, "hunter2"));

    let checked = integration.validate().await.expect("the plugin ran");
    assert!(checked.is_ok(), "{checked:?}");

    let received = hook.received();
    assert_eq!(received.len(), 1, "{received:?}");
    let request = &received[0];
    assert!(request.starts_with("POST /hook "), "{request}");
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer hunter2"),
        "{request}"
    );
    assert!(request.ends_with(r#"{"event":"hello"}"#), "{request}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_address_that_refuses_fails_the_check() {
    let hook = Hook::start(500).await;
    let integration = Integration::new(webhook(), answers(&hook.url, ""));

    let checked = integration.validate().await.expect("the plugin ran");
    assert!(
        matches!(&checked, Err(wit::SourceError::Backend(message)) if message.contains("500")),
        "{checked:?}"
    );
    assert!(
        !hook.received()[0]
            .to_ascii_lowercase()
            .contains("authorization:"),
        "an empty token sends none"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_closed_integration_stays_closed() {
    let hook = Hook::start(200).await;
    let integration = Integration::new(webhook(), answers(&hook.url, ""));
    for _ in 0..3 {
        assert!(integration.validate().await.expect("ran").is_ok());
    }
    integration.close().await;
    assert!(integration.is_closed());
    assert!(
        integration.validate().await.is_err(),
        "a call after closing opens nothing"
    );
    assert_eq!(hook.received().len(), 3);
}
