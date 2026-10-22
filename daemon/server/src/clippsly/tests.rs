use super::*;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Reply {
    path: String,
    status: u16,
    body: Value,
    upload: bool,
}

impl Reply {
    fn get(path: &str, body: Value) -> Self {
        Self {
            path: path.into(),
            status: 200,
            body,
            upload: false,
        }
    }
}

async fn mock(replies: Vec<Reply>) -> (ClippslyClient, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = ClippslyClient::new(
        &format!("http://{}", listener.local_addr().unwrap()),
        "test-app-password",
    )
    .unwrap();
    let server = tokio::spawn(async move {
        for reply in replies {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut bytes = Vec::new();
            let end = loop {
                let mut chunk = [0; 8192];
                let read = socket.read(&mut chunk).await.unwrap();
                assert!(read > 0);
                bytes.extend_from_slice(&chunk[..read]);
                if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
            let method = if reply.upload { "POST" } else { "GET" };
            assert!(
                headers.starts_with(&format!("{method} {} HTTP/1.1\r\n", reply.path)),
                "{headers}"
            );
            assert!(
                headers
                    .to_ascii_lowercase()
                    .contains("authorization: bearer test-app-password\r\n")
            );
            assert!(
                !headers
                    .lines()
                    .next()
                    .unwrap()
                    .contains("test-app-password")
            );
            let length = headers
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                .map(|(_, length)| length.trim().parse::<usize>().unwrap())
                .unwrap_or(0);
            while bytes.len() < end + length {
                let mut chunk = [0; 8192];
                let read = socket.read(&mut chunk).await.unwrap();
                assert!(read > 0);
                bytes.extend_from_slice(&chunk[..read]);
            }
            if reply.upload {
                let body = String::from_utf8_lossy(&bytes[end..]);
                assert!(body.contains("name=\"audio\"; filename=\"song.flac\""));
                assert!(body.contains("fLaC-test"));
            }
            let body = reply.body.to_string();
            socket.write_all(format!("HTTP/1.1 {} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", reply.status, body.len()).as_bytes()).await.unwrap();
        }
    });
    (client, server)
}

fn track(id: u64) -> Value {
    json!({"id": id, "title": "Song", "artist": "Artist", "duration_seconds": 12.5, "album_id": 7, "album_title": "Album"})
}

#[tokio::test]
async fn clippsly_login_uses_the_app_password_and_account_identity() {
    let (client, server) = mock(vec![Reply::get(
        "/v1/account",
        json!({"account": {"id": 42, "username": "alice"}}),
    )])
    .await;
    let auth = crate::provider::ProviderClient::new(
        config::MusicService::Clippsly,
        client.base.to_string(),
        "test-device",
    )
    .login("alice", "test-app-password")
    .await
    .unwrap();
    assert_eq!(auth.user_id, "42");
    assert_eq!(auth.access_token, "test-app-password");
    server.await.unwrap();
}

#[tokio::test]
async fn clippsly_pages_tracks_without_silently_accepting_a_broken_page() {
    let first: Vec<Value> = (1..=200).map(track).collect();
    let (client, server) = mock(vec![
        Reply::get(
            "/v1/library/tracks?limit=200&offset=0",
            json!({"tracks": first}),
        ),
        Reply::get(
            "/v1/library/tracks?limit=200&offset=200",
            json!({"tracks": [track(201)]}),
        ),
    ])
    .await;
    let tracks = client.tracks().await.unwrap();
    assert_eq!(tracks.len(), 201);
    assert_eq!(tracks.last().unwrap().id, 201);
    server.await.unwrap();

    for broken in [
        json!({"success": true}),
        json!({"tracks": [{}]}),
        json!({"success": false, "tracks": []}),
    ] {
        let (client, server) = mock(vec![Reply::get(
            "/v1/library/tracks?limit=200&offset=0",
            broken,
        )])
        .await;
        assert!(client.tracks().await.is_err());
        server.await.unwrap();
    }
}

#[tokio::test]
async fn clippsly_rejects_a_server_that_repeats_pages() {
    let tracks: Vec<Value> = (1..=200).map(track).collect();
    let (client, server) = mock(vec![
        Reply::get(
            "/v1/library/tracks?limit=200&offset=0",
            json!({"tracks": tracks}),
        ),
        Reply::get(
            "/v1/library/tracks?limit=200&offset=200",
            json!({"tracks": [track(1)]}),
        ),
    ])
    .await;
    assert!(
        client
            .tracks()
            .await
            .unwrap_err()
            .to_string()
            .contains("repeated")
    );
    server.await.unwrap();
}

#[tokio::test]
async fn clippsly_auth_and_rate_limits_are_reported_without_response_secrets() {
    for status in [401, 403, 429, 500] {
        let (client, server) = mock(vec![Reply {
            path: "/v1/account".into(),
            status,
            body: json!({"error": "test-app-password"}),
            upload: false,
        }])
        .await;
        let error = client.account().await.err().unwrap();
        if matches!(status, 401 | 403) {
            assert_eq!(error, SourceError::Auth);
        }
        assert!(!error.to_string().contains("test-app-password"));
        server.await.unwrap();
    }
}

#[tokio::test]
async fn clippsly_resolves_signed_streams_and_relative_covers() {
    let (client, server) = mock(vec![Reply::get("/v1/tracks/42/stream?variant=best", json!({"stream": {"url": "/proxy/42", "direct_url": "https://cdn.example/song?signature=abc", "duration_seconds": 12.5, "bitrate": 320000}}))]).await;
    let stream = client.stream("42").await.unwrap();
    assert_eq!(
        client.stream_url(&stream).unwrap(),
        "https://cdn.example/song?signature=abc"
    );
    assert!(
        client
            .media_url("/covers/42")
            .unwrap()
            .ends_with("/covers/42")
    );
    assert!(client.media_url("file:///etc/passwd").is_none());
    assert!(client.stream("../account").await.is_err());
    server.await.unwrap();
}

#[tokio::test]
async fn clippsly_uploads_multipart_audio_after_checking_quota() {
    let (client, server) = mock(vec![
        Reply::get("/v1/account/storage", json!({"track_count": 1, "used_bytes": 10, "quota_bytes": 100, "quota_remaining_bytes": 90})),
        Reply { path: "/v1/upload".into(), status: 201, body: json!({"success": true}), upload: true },
    ]).await;
    client
        .upload("song.flac".into(), b"fLaC-test".to_vec())
        .await
        .unwrap();
    server.await.unwrap();

    let (client, server) = mock(vec![Reply::get(
        "/v1/account/storage",
        json!({"track_count": 1, "used_bytes": 99, "quota_bytes": 100, "quota_remaining_bytes": 1}),
    )])
    .await;
    assert!(matches!(
        client
            .upload("song.flac".into(), b"fLaC-test".to_vec())
            .await,
        Err(SourceError::InvalidInput(_))
    ));
    server.await.unwrap();
}

#[test]
fn clippsly_upload_validation_rejects_paths_empty_files_and_unsupported_formats() {
    assert!(validate_upload("song.FLAC", 1).is_ok());
    for (name, size) in [
        ("song.flac", 0),
        ("song.flac", MAX_UPLOAD_BYTES + 1),
        ("../song.flac", 1),
        ("song.txt", 1),
        ("song\n.flac", 1),
    ] {
        assert!(validate_upload(name, size).is_err());
    }
}
