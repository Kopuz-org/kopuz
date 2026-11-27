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
        Self::answering(status, "").await
    }

    /// Answers every request with `status` and `body`.
    async fn answering(status: u16, body: &'static str) -> Self {
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
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
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

#[tokio::test(flavor = "multi_thread")]
async fn a_call_dropped_midway_leaves_no_half_run_instance_behind() {
    // The first request is never answered; every later one is.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let url = format!("http://{}/hook", listener.local_addr().expect("address"));
    let arrived = Arc::new(tokio::sync::Notify::new());
    let told = Arc::clone(&arrived);
    tokio::spawn(async move {
        let mut first = true;
        let mut held = Vec::new();
        while let Ok((mut stream, _)) = listener.accept().await {
            read_request(&mut stream).await;
            if std::mem::take(&mut first) {
                held.push(stream);
                told.notify_one();
                continue;
            }
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await;
        }
    });
    let integration = Integration::new(webhook(), answers(&url, ""));
    // Given up on only once the plugin is inside its call, waiting on the address.
    tokio::select! {
        _ = integration.validate() => panic!("the first check should hang"),
        () = arrived.notified() => {}
    }
    let again = integration.validate().await.expect("the plugin ran");
    assert!(again.is_ok(), "a fresh instance answers: {again:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_listening_integration_is_told_what_plays() {
    let hook = Hook::start(200).await;
    let integration = Integration::new(webhook(), answers(&hook.url, ""));
    let started = wit::PlaybackEvent::Started(wit::Started {
        track: wit::NowPlaying {
            key: "subsonic:tr-1".into(),
            title: "Blue in Green".into(),
            artist: "Miles Davis".into(),
            album: "Kind of Blue".into(),
            duration_ms: Some(337_000),
            musicbrainz_recording_id: None,
            musicbrainz_release_id: None,
        },
        position_ms: 0,
    });
    integration.on_playback(&started).await.expect("told");
    integration
        .on_playback(&wit::PlaybackEvent::Paused(12_000))
        .await
        .expect("told");
    integration
        .on_playback(&wit::PlaybackEvent::Stopped)
        .await
        .expect("told");

    let bodies: Vec<serde_json::Value> = hook
        .received()
        .iter()
        .map(|request| {
            let body = request.split("\r\n\r\n").nth(1).unwrap_or_default();
            serde_json::from_str(body).expect("a JSON body")
        })
        .collect();
    assert_eq!(bodies.len(), 3, "{bodies:?}");
    assert_eq!(bodies[0]["event"], "started");
    assert_eq!(bodies[0]["track"]["key"], "subsonic:tr-1");
    assert_eq!(bodies[0]["track"]["title"], "Blue in Green");
    assert_eq!(
        bodies[1],
        serde_json::json!({ "event": "paused", "position_ms": 12000 })
    );
    assert_eq!(bodies[2], serde_json::json!({ "event": "stopped" }));
}

fn frame(level: f32, beat: bool) -> wit::AudioFrame {
    wit::AudioFrame {
        level,
        peak: level,
        low: level,
        mid: level,
        high: level,
        onset: if beat { level } else { 0.0 },
        beat,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_integration_hears_the_beats_it_asked_for() {
    let hook = Hook::start(200).await;
    let mut settings = answers(&hook.url, "");
    let quiet = Integration::new(webhook(), settings.clone());
    quiet
        .on_audio(&[frame(0.4, true), frame(0.1, false)])
        .await
        .expect("told");
    assert!(hook.received().is_empty(), "beats are off unless asked for");

    settings.push(("beats".to_string(), "true".to_string()));
    let beating = Integration::new(webhook(), settings);
    beating
        .on_audio(&[frame(0.1, false), frame(0.5, true), frame(0.2, false)])
        .await
        .expect("told");
    let received = hook.received();
    assert_eq!(received.len(), 1, "only the beat is posted: {received:?}");
    let body = received[0].split("\r\n\r\n").nth(1).unwrap_or_default();
    let beat: serde_json::Value = serde_json::from_str(body).expect("JSON");
    assert_eq!(beat["event"], "beat");
    assert_eq!(beat["level"], 0.5);
}

/// What reached "kopuz" through the bridge, in order.
#[derive(Default)]
struct FakeKopuz {
    seen: Mutex<Vec<String>>,
}

impl plugin_host::Bridge for FakeKopuz {
    fn control(&self, command: wit::playback::Command) -> plugin_host::Answer<()> {
        self.seen.lock().unwrap().push(format!("{command:?}"));
        Box::pin(async { Ok(()) })
    }

    fn queue(&self, keys: Vec<String>, mode: wit::playback::QueueMode) -> plugin_host::Answer<()> {
        self.seen
            .lock()
            .unwrap()
            .push(format!("queue {mode:?} {keys:?}"));
        Box::pin(async { Ok(()) })
    }

    fn search(&self, query: String, limit: u32) -> plugin_host::Answer<Vec<wit::library::Track>> {
        self.seen
            .lock()
            .unwrap()
            .push(format!("search {query} {limit}"));
        let found = ["blue-1", "blue-2"].map(library_track).to_vec();
        Box::pin(async move { Ok(found) })
    }

    fn tracks(&self, keys: Vec<String>) -> plugin_host::Answer<Vec<wit::library::Track>> {
        let found = keys
            .iter()
            .map(|key| wit::library::Track {
                artist: "Miles Davis".into(),
                ..library_track(key)
            })
            .collect();
        Box::pin(async move { Ok(found) })
    }

    fn album_tracks(&self, _: String) -> plugin_host::Answer<Vec<wit::library::Track>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn tracks_page(&self, _: u32, _: u32) -> plugin_host::Answer<wit::library::TrackPage> {
        Box::pin(async {
            Ok(wit::library::TrackPage {
                total: 0,
                tracks: Vec::new(),
            })
        })
    }

    fn playlists(&self) -> plugin_host::Answer<Vec<wit::library::Playlist>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn favorites(&self) -> plugin_host::Answer<Vec<String>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

fn library_track(key: &str) -> wit::library::Track {
    wit::library::Track {
        key: key.to_string(),
        title: key.to_string(),
        artist: String::new(),
        album: String::new(),
        album_id: String::new(),
        duration_ms: None,
        track_number: None,
        disc_number: None,
        musicbrainz_recording_id: None,
        musicbrainz_release_id: None,
    }
}

/// The remote example, without the manifest lines in `dropped`.
fn remote(dropped: &[&str]) -> Arc<Plugin> {
    let bundled = bundled_plugins::examples()
        .iter()
        .find(|example| example.manifest.contains("id = \"remote\""))
        .expect("the remote example is built with the examples feature");
    let text = dropped
        .iter()
        .fold(bundled.manifest.to_string(), |text, line| {
            text.replace(line, "")
        });
    let manifest = Manifest::parse(&text).expect("its manifest");
    Arc::new(Plugin::new(
        manifest,
        Code::Wasm(std::borrow::Cow::Borrowed(
            bundled.wasm.expect("a component"),
        )),
        Origin::Installed,
    ))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_allowed_to_steer_playback_does_on_its_tick() {
    let orders = Hook::answering(
        200,
        "next\nvolume 50\nplay key one\nqueue key two\nvolume nan\n",
    )
    .await;
    let kopuz = Arc::new(FakeKopuz::default());
    let integration = Integration::new(remote(&[]), vec![("url".into(), orders.url.clone())])
        .with_bridge(kopuz.clone());
    integration.on_tick().await.expect("ticked");
    assert_eq!(
        *kopuz.seen.lock().unwrap(),
        [
            "Command::Next".to_string(),
            "Command::Volume(0.5)".to_string(),
            r#"queue QueueMode::Replace ["key one"]"#.to_string(),
            r#"queue QueueMode::Append ["key two"]"#.to_string(),
        ],
        "a volume that is not a number never reaches kopuz"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_that_did_not_ask_is_refused_playback() {
    let orders = Hook::answering(200, "next\n").await;
    let kopuz = Arc::new(FakeKopuz::default());
    let integration = Integration::new(
        remote(&["playback = \"control\"\n"]),
        vec![("url".into(), orders.url.clone())],
    )
    .with_bridge(kopuz.clone());
    integration.on_tick().await.expect("ticked");
    assert_eq!(orders.received().len(), 1, "it asked its address");
    assert!(
        kopuz.seen.lock().unwrap().is_empty(),
        "nothing reached kopuz"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_allowed_to_read_the_library_finds_and_plays() {
    let orders = Hook::answering(200, "find blue\n").await;
    let kopuz = Arc::new(FakeKopuz::default());
    let integration = Integration::new(remote(&[]), vec![("url".into(), orders.url.clone())])
        .with_bridge(kopuz.clone());
    integration.on_tick().await.expect("ticked");
    assert_eq!(
        *kopuz.seen.lock().unwrap(),
        [
            "search blue 50".to_string(),
            r#"queue QueueMode::Replace ["blue-1", "blue-2"]"#.to_string(),
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_that_did_not_ask_is_refused_the_library() {
    let orders = Hook::answering(200, "find blue\n").await;
    let kopuz = Arc::new(FakeKopuz::default());
    let integration = Integration::new(
        remote(&["library = \"read\"\n"]),
        vec![("url".into(), orders.url.clone())],
    )
    .with_bridge(kopuz.clone());
    integration.on_tick().await.expect("ticked");
    assert!(
        kopuz.seen.lock().unwrap().is_empty(),
        "nothing was searched, so nothing was queued"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_action_runs_with_what_the_person_typed_or_picked() {
    let orders = Hook::answering(200, "").await;
    let kopuz = Arc::new(FakeKopuz::default());
    let integration = Integration::new(remote(&[]), vec![("url".into(), orders.url.clone())])
        .with_bridge(kopuz.clone());

    let found = integration
        .run_action("find", &wit::ActionTarget::None, Some("blue"))
        .await
        .expect("ran");
    assert_eq!(
        found,
        Ok(Some("Playing what the library has for blue".into()))
    );

    let more = integration
        .run_action(
            "more-by-artist",
            &wit::ActionTarget::Tracks(vec!["kind-of-blue-1".into()]),
            None,
        )
        .await
        .expect("ran");
    assert_eq!(more, Ok(Some("Playing more by Miles Davis".into())));
    assert_eq!(
        *kopuz.seen.lock().unwrap(),
        [
            "search blue 50".to_string(),
            r#"queue QueueMode::Replace ["blue-1", "blue-2"]"#.to_string(),
            "search Miles Davis 50".to_string(),
            r#"queue QueueMode::Replace ["blue-1", "blue-2"]"#.to_string(),
        ]
    );

    let blank = integration
        .run_action("find", &wit::ActionTarget::None, Some("  "))
        .await
        .expect("ran");
    assert_eq!(blank, Err("type something to search for".into()));
}
