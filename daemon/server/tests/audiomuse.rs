//! The AudioMuse-AI client against a local fake server.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use server::audiomuse::{AudioMuse, AudioMuseError, SimilarArtist};
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
    let mix = client.fingerprint(5).await.expect("fingerprint");
    assert!(!mix.is_empty());

    let seed = &mix[0];
    assert!(
        !client
            .similar_tracks(seed, 5)
            .await
            .expect("similar tracks")
            .is_empty()
    );
    let score = client
        .score(seed)
        .await
        .expect("score")
        .expect("an analysed song");
    assert!(!score.tags.is_empty() && !score.moods.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn sound_calls_send_their_query_and_an_unanalysed_song_is_empty() {
    let mut known = routes();
    known.insert(
        "/api/similar_tracks",
        (
            200,
            r#"[{"item_id":"b","distance":0.1},{"item_id":"c","distance":0.4}]"#.to_string(),
        ),
    );
    known.insert(
        "/api/similar_artists",
        (
            200,
            r#"[{"artist":"Boris","artist_id":"ar-2","divergence":0.2,"track_count":3}]"#
                .to_string(),
        ),
    );
    known.insert(
        "/external/get_score",
        (
            200,
            r#"{"item_id":"a","mood_vector":"metal:0.6","other_features":"sad:0.5"}"#.to_string(),
        ),
    );
    let fake = FakeServer::start(known).await;
    let client = AudioMuse::new(&fake.url, TOKEN).expect("client");

    assert_eq!(
        client.similar_tracks("a b", 20).await.expect("tracks"),
        [("b".to_string(), 0.1), ("c".to_string(), 0.4)]
    );
    assert_eq!(
        client.similar_artists("ar-1", 20).await.expect("artists"),
        [SimilarArtist {
            id: "ar-2".into(),
            name: "Boris".into(),
            divergence: 0.2,
            tracks: Some(3),
        }]
    );
    let score = client.score("a").await.expect("score").expect("analysed");
    assert_eq!(score.tags, [("metal".to_string(), 0.6)]);
    assert_eq!(score.moods, [("sad".to_string(), 0.5)]);

    let lines: Vec<String> = fake
        .requests()
        .iter()
        .map(|request| request.lines().next().unwrap_or_default().to_string())
        .collect();
    assert_eq!(
        lines,
        [
            "GET /api/similar_tracks?item_id=a+b&n=20 HTTP/1.1",
            "GET /api/similar_artists?artist_id=ar-1&n=20 HTTP/1.1",
            "GET /external/get_score?id=a HTTP/1.1",
        ]
    );

    let none = FakeServer::start(HashMap::new()).await;
    let client = AudioMuse::new(&none.url, TOKEN).expect("client");
    assert_eq!(client.score("a").await.expect("404"), None);
    assert!(client.similar_tracks("a", 5).await.expect("404").is_empty());
    assert!(
        client
            .similar_artists("a", 5)
            .await
            .expect("404")
            .is_empty()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sound_answer_of_another_shape_is_malformed() {
    let mut odd = routes();
    odd.insert("/external/get_score", (200, "<html></html>".to_string()));
    let fake = FakeServer::start(odd).await;
    let client = AudioMuse::new(&fake.url, TOKEN).expect("client");

    assert!(matches!(
        client.score("a").await,
        Err(AudioMuseError::Malformed)
    ));
}
