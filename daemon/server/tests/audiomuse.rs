//! The AudioMuse-AI client against a local fake server.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use server::audiomuse::{AudioMuse, AudioMuseError};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

const TOKEN: &str = "test-token";

/// A canned answer per path; every request is kept whole, lowercased headers included.
struct FakeServer {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
}

fn complete(request: &[u8]) -> bool {
    let text = String::from_utf8_lossy(request).to_lowercase();
    let Some((head, body)) = text.split_once("\r\n\r\n") else {
        return false;
    };
    let wanted = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    body.len() >= wanted
}

impl FakeServer {
    async fn start(routes: HashMap<&'static str, (u16, String)>) -> Self {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind");
        let url = format!("http://{}", listener.local_addr().expect("address"));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&requests);
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut request = Vec::new();
                let mut buffer = [0u8; 4096];
                while !complete(&request) {
                    match stream.read(&mut buffer).await {
                        Ok(0) | Err(_) => break,
                        Ok(read) => request.extend_from_slice(&buffer[..read]),
                    }
                }
                let text = String::from_utf8_lossy(&request).into_owned();
                let target = text.split_whitespace().nth(1).unwrap_or_default();
                let path = target.split('?').next().unwrap_or_default();
                let (status, body) = routes
                    .get(path)
                    .cloned()
                    .unwrap_or((404, "<html></html>".into()));
                seen.lock().unwrap().push(text.clone());
                let reply = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes()).await;
            }
        });
        Self { url, requests }
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

fn routes() -> HashMap<&'static str, (u16, String)> {
    HashMap::from([
        ("/api/health", (200, r#"{"status":"ok"}"#.to_string())),
        (
            "/api/version",
            (200, r#"{"app_version":"v3.7.1"}"#.to_string()),
        ),
        (
            "/api/clap/search",
            (
                200,
                r#"{"count":2,"query":"q","results":[{"item_id":"b"},{"item_id":"a"}]}"#
                    .to_string(),
            ),
        ),
        (
            "/api/sonic_fingerprint/generate",
            (200, r#"[{"item_id":"x"},{"item_id":"y"}]"#.to_string()),
        ),
    ])
}

#[tokio::test(flavor = "multi_thread")]
async fn every_call_sends_the_bearer_token_to_its_path() {
    let fake = FakeServer::start(routes()).await;
    let client = AudioMuse::new(&format!("{}/", fake.url), TOKEN).expect("client");

    client.health().await.expect("health");
    assert_eq!(client.version().await.expect("version"), "v3.7.1");
    assert_eq!(
        client.describe("calm piano", 7).await.expect("describe"),
        ["b", "a"]
    );
    assert_eq!(
        client.fingerprint(50).await.expect("fingerprint"),
        ["x", "y"]
    );

    let requests = fake.requests();
    let lines: Vec<&str> = requests
        .iter()
        .map(|request| request.lines().next().unwrap_or_default())
        .collect();
    assert_eq!(
        lines,
        [
            "GET /api/health HTTP/1.1",
            "GET /api/version HTTP/1.1",
            "POST /api/clap/search HTTP/1.1",
            "GET /api/sonic_fingerprint/generate?n=50 HTTP/1.1",
        ]
    );
    for request in &requests {
        assert!(
            request
                .to_lowercase()
                .contains(&format!("authorization: bearer {TOKEN}")),
            "{request}"
        );
    }
    assert!(
        requests[2].contains(r#""query":"calm piano""#),
        "{}",
        requests[2]
    );
    assert!(requests[2].contains(r#""limit":7"#), "{}", requests[2]);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_path_is_a_status_and_a_bad_token_is_rejected() {
    let mut denied = routes();
    denied.insert("/api/health", (401, String::new()));
    let fake = FakeServer::start(denied).await;
    let client = AudioMuse::new(&fake.url, TOKEN).expect("client");

    assert!(matches!(
        client.health().await,
        Err(AudioMuseError::Rejected)
    ));

    let empty = FakeServer::start(HashMap::new()).await;
    let client = AudioMuse::new(&empty.url, TOKEN).expect("client");
    assert!(matches!(
        client.version().await,
        Err(AudioMuseError::Status(404))
    ));
}

/// Against a real instance: `KOPUZ_AUDIOMUSE_URL=… KOPUZ_AUDIOMUSE_TOKEN=… cargo test -p kopuz-server --test audiomuse -- --ignored`.
#[tokio::test]
#[ignore = "needs a running AudioMuse-AI"]
async fn a_live_instance_answers_every_call() {
    let (Ok(url), Ok(token)) = (
        std::env::var("KOPUZ_AUDIOMUSE_URL"),
        std::env::var("KOPUZ_AUDIOMUSE_TOKEN"),
    ) else {
        panic!("set KOPUZ_AUDIOMUSE_URL and KOPUZ_AUDIOMUSE_TOKEN");
    };
    let client = AudioMuse::new(&url, &token).expect("client");

    client.health().await.expect("health");
    assert!(!client.version().await.expect("version").is_empty());
    assert!(
        !client
            .describe("calm piano", 5)
            .await
            .expect("describe")
            .is_empty()
    );
    assert!(!client.fingerprint(5).await.expect("fingerprint").is_empty());
}
