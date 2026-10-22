//! The official plugins, run end to end: the embedded component, loaded by the
//! plugin host, answering through the `MediaSource` facade against a fake
//! server on a local port. What a plugin does on the wire and what the host
//! makes of its answer are both covered without a real account.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use config::{MusicServer, MusicService};
use db::Source;
use server::source::{self, AuthOutcome, PlaylistOps, SourceError};
use server::sources::Sources;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn unique_db() -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("kopuz-plugin-source-{}-{seq}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("kopuz.db")
}

/// A canned answer per request path; anything else is a 404.
struct FakeServer {
    url: String,
    /// Each request's path and query, in arrival order.
    requests: Arc<Mutex<Vec<String>>>,
}

impl FakeServer {
    async fn start(routes: HashMap<&'static str, (u16, String)>) -> Self {
        Self::start_on("127.0.0.1", routes).await
    }

    async fn start_on(host: &str, routes: HashMap<&'static str, (u16, String)>) -> Self {
        let listener = tokio::net::TcpListener::bind((host, 0))
            .await
            .expect("bind");
        let address = listener.local_addr().expect("address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&requests);
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut buffer = vec![0u8; 16 * 1024];
                let read = stream.read(&mut buffer).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
                let target = request
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or_default()
                    .to_string();
                seen.lock().unwrap().push(target.clone());
                let path = target.split('?').next().unwrap_or_default();
                let (status, body) = routes.get(path).cloned().unwrap_or((404, String::new()));
                let reply = if (300..400).contains(&status) {
                    format!(
                        "HTTP/1.1 {status} Redirect\r\nLocation: {body}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                } else {
                    format!(
                        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                };
                let _ = stream.write_all(reply.as_bytes()).await;
            }
        });
        Self {
            url: format!("http://{address}"),
            requests,
        }
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

fn subsonic_ok(data: serde_json::Value) -> (u16, String) {
    let mut response = serde_json::json!({ "status": "ok", "version": "1.16.1" });
    if let (Some(response), Some(data)) = (response.as_object_mut(), data.as_object()) {
        response.extend(data.clone());
    }
    (
        200,
        serde_json::json!({ "subsonic-response": response }).to_string(),
    )
}

fn sources_for(service: MusicService, url: &str) -> (Sources, Source) {
    let mut server = MusicServer::new_with_service("test".into(), url.into(), service);
    server.access_token = Some("hunter2".into());
    server.user_id = Some("someone".into());
    let mut sources = Sources::default();
    sources.set_active_server(server);
    let active = sources.active.clone();
    (sources, active)
}

fn subsonic_library() -> HashMap<&'static str, (u16, String)> {
    HashMap::from([
        ("/rest/ping.view", subsonic_ok(serde_json::json!({}))),
        (
            "/rest/getArtists.view",
            subsonic_ok(serde_json::json!({
                "artists": { "index": [{ "artist": [{ "id": "ar-1", "name": "Ada", "coverArt": "ar-1" }] }] }
            })),
        ),
        (
            "/rest/getAlbumList2.view",
            subsonic_ok(serde_json::json!({
                "albumList2": { "album": [{
                    "id": "al-1", "name": "First", "artist": "Ada", "artistId": "ar-1",
                    "year": 2001, "genre": "Jazz", "coverArt": "al-1"
                }] }
            })),
        ),
        (
            "/rest/getAlbum.view",
            subsonic_ok(serde_json::json!({
                "album": { "song": [
                    { "id": "s-1", "title": "One", "duration": 61, "track": 1, "bitRate": 320 },
                    { "id": "s-2", "title": "Two", "artist": "Boris", "duration": 62, "track": 2 }
                ] }
            })),
        ),
        (
            "/rest/updatePlaylist.view",
            subsonic_ok(serde_json::json!({})),
        ),
    ])
}

#[tokio::test(flavor = "multi_thread")]
async fn the_subsonic_plugin_syncs_a_library_through_the_facade() {
    let server = FakeServer::start(subsonic_library()).await;
    let db = db::init(&unique_db()).await.unwrap();
    let (sources, active) = sources_for(MusicService::Subsonic, &server.url);
    let src = source::resolve(db, &sources, &active);

    let caps = src.capabilities();
    assert!(caps.sync && caps.downloads && caps.play_queue);
    assert!(caps.radio.track && !caps.radio.playlist);
    assert_eq!(caps.playlists, PlaylistOps::Reorder);

    assert_eq!(src.validate().await, AuthOutcome::Valid);

    let library = src.fetch_library().await.expect("library");
    assert_eq!(library.albums.len(), 1);
    let album = &library.albums[0];
    assert_eq!(album.id, "subsonic:al-1");
    assert_eq!(album.genre, "Jazz");
    assert_eq!(album.year, 2001);

    let titles: Vec<_> = library.tracks.iter().map(|t| t.title.as_str()).collect();
    assert_eq!(titles, ["One", "Two"]);
    let first = &library.tracks[0];
    assert_eq!(first.id.uid(), "subsonic:s-1");
    assert_eq!(first.album_id, "subsonic:al-1");
    assert_eq!(
        first.artist, "Ada",
        "a song without an artist takes the album's"
    );
    assert_eq!(first.album, "First");
    assert_eq!(first.bitrate, 320);
    assert_eq!(library.tracks[1].artist, "Boris");
    assert_eq!(library.artists.len(), 1);
    assert_eq!(library.artists[0].0.id.as_deref(), Some("ar-1"));

    let every_request_signed = server
        .requests()
        .iter()
        .all(|request| request.contains("u=someone") && request.contains("t="));
    assert!(every_request_signed, "{:?}", server.requests());
    assert!(
        !server.requests().iter().any(|r| r.contains("hunter2")),
        "the password is hashed, never sent"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_custom_server_files_its_items_under_its_own_service() {
    let server = FakeServer::start(subsonic_library()).await;
    let db = db::init(&unique_db()).await.unwrap();
    let (sources, active) = sources_for(MusicService::Custom, &server.url);
    let src = source::resolve(db, &sources, &active);

    let library = src.fetch_library().await.expect("library");
    assert_eq!(library.albums[0].id, "custom:al-1");
    assert_eq!(library.tracks[0].id.uid(), "custom:s-1");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_sign_in_reads_as_expired_and_a_dead_server_as_unreachable() {
    let refused = FakeServer::start(HashMap::from([(
        "/rest/ping.view",
        (
            200,
            r#"{"subsonic-response":{"status":"failed","version":"1.16.1","error":{"code":40,"message":"Wrong username or password"}}}"#.to_string(),
        ),
    )]))
    .await;
    let db = db::init(&unique_db()).await.unwrap();
    let (sources, active) = sources_for(MusicService::Subsonic, &refused.url);
    let src = source::resolve(db.clone(), &sources, &active);
    assert_eq!(src.validate().await, AuthOutcome::Expired);

    let closed = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = closed.local_addr().expect("address");
    drop(closed);
    let (sources, active) = sources_for(MusicService::Subsonic, &format!("http://{address}"));
    let src = source::resolve(db, &sources, &active);
    assert_eq!(src.validate().await, AuthOutcome::Unreachable);
    assert_eq!(
        src.fetch_playlists().await.err(),
        Some(SourceError::Connectivity)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn signing_in_goes_through_the_plugin() {
    let server = FakeServer::start(subsonic_library()).await;
    let client =
        server::provider::ProviderClient::new(MusicService::Subsonic, server.url.clone(), "dev");
    let session = client.login("someone", "hunter2").await.expect("signed in");
    assert_eq!(session.access_token, "hunter2");
    assert_eq!(session.user_id, "someone");

    let refused =
        FakeServer::start(HashMap::from([("/rest/ping.view", (401, String::new()))])).await;
    let client = server::provider::ProviderClient::new(MusicService::Subsonic, refused.url, "dev");
    assert_eq!(
        client.login("someone", "wrong").await.err(),
        Some(SourceError::Auth)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_redirect_off_the_configured_server_is_refused() {
    let elsewhere = FakeServer::start_on(
        "127.0.0.2",
        HashMap::from([("/rest/ping.view", subsonic_ok(serde_json::json!({})))]),
    )
    .await;
    let bouncing = FakeServer::start(HashMap::from([(
        "/rest/ping.view",
        (302, format!("{}/rest/ping.view", elsewhere.url)),
    )]))
    .await;
    let db = db::init(&unique_db()).await.unwrap();
    let (sources, active) = sources_for(MusicService::Subsonic, &bouncing.url);
    let src = source::resolve(db, &sources, &active);

    assert_ne!(src.validate().await, AuthOutcome::Valid);
    assert_eq!(bouncing.requests().len(), 1);
    assert!(
        elsewhere.requests().is_empty(),
        "the plugin never reached a host its manifest does not allow"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_playlist_add_lands_on_the_server_and_in_the_cache() {
    let server = FakeServer::start(subsonic_library()).await;
    let db = db::init(&unique_db()).await.unwrap();
    let (sources, active) = sources_for(MusicService::Subsonic, &server.url);
    let src = source::resolve(db.clone(), &sources, &active);

    db.upsert_playlist_meta(&active, "pl-1", "Mix", None, None)
        .await
        .unwrap();
    let landed = src
        .add_to_playlist("pl-1", &["s-1".into(), "s-2".into()])
        .await
        .expect("added");
    assert_eq!(landed, ["s-1", "s-2"]);
    let adds = server
        .requests()
        .iter()
        .filter(|r| r.starts_with("/rest/updatePlaylist.view") && r.contains("songIdToAdd"))
        .count();
    assert_eq!(adds, 2);

    let store = db.load_playlists(&active).await.unwrap();
    let playlist = store
        .playlists
        .iter()
        .find(|p| p.id == "pl-1")
        .expect("playlist cached");
    assert_eq!(playlist.tracks, ["s-1", "s-2"]);
}

#[test]
fn every_built_in_service_has_a_plugin() {
    let registry = server::plugins::registry();
    for service in MusicService::ALL {
        let plugin = registry
            .for_service(service.id())
            .unwrap_or_else(|| panic!("no plugin serves {}", service.id()));
        let native = matches!(
            service,
            MusicService::YtMusic
                | MusicService::AppleMusic
                | MusicService::Spotify
                | MusicService::Nextcloud
                | MusicService::Smb
        );
        assert_eq!(plugin.is_native(), native, "{}", service.id());
    }
}
