//! Contract tests: the same assertions run through `LocalApi` (in-process)
//! and `HttpApi` (over a real axum server + SSE), proving the two transports
//! cannot drift. This is the parity mechanism the split relies on.

use std::io::Cursor;
use std::sync::Arc;
use std::time::{Duration, Instant};

use api::{
    ApiError, ApiEvent, ErrorCode, Intent, KopuzApi, LoopMode, Page, Phase, PlayerCommand,
    PlayerState, QueueContext, QueueEdit, QueueMode, SetQueueRequest, TrackFilter,
};
use daemon::session::FactoryOverride;
use daemon::{LocalApi, PlaybackServices, QueueMaterializer, SessionHandle};
use player::engine::{NullSink, SourceFactory};
use player::player::Player;
use reader::Track;

struct StubLibrary;

#[async_trait::async_trait]
impl QueueMaterializer for StubLibrary {
    async fn materialize(&self, context: &QueueContext) -> Result<Vec<Track>, ApiError> {
        match context {
            QueueContext::Tracks { keys } => Ok(keys.iter().map(|key| track(key)).collect()),
            _ => Err(ApiError::unsupported("stub resolves raw tracks only")),
        }
    }
}

fn track(key: &str) -> Track {
    Track {
        id: reader::models::TrackId::Local(std::path::PathBuf::from(key)),
        cover: None,
        album_id: String::new(),
        title: key.to_string(),
        artist: String::new(),
        album: String::new(),
        duration: 6,
        khz: 44,
        bitrate: 320,
        track_number: None,
        disc_number: None,
        musicbrainz_release_id: None,
        musicbrainz_recording_id: None,
        musicbrainz_track_id: None,
        playlist_item_id: None,
        artists: vec![],
    }
}

fn wav_bytes(seconds: u64) -> Vec<u8> {
    let sample_rate: u32 = 44_100;
    let channels: usize = 2;
    let frames = seconds as usize * sample_rate as usize;
    let data_len = frames * channels * 2;
    let mut bytes = Vec::with_capacity(44 + data_len);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&(channels as u16).to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * channels as u32 * 2).to_le_bytes());
    bytes.extend_from_slice(&((channels * 2) as u16).to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(data_len as u32).to_le_bytes());
    bytes.resize(44 + data_len, 0);
    bytes
}

fn wav_factory(seconds: u64) -> SourceFactory {
    let bytes = wav_bytes(seconds);
    Box::new(move || Ok(player::decoder::from_stream(Cursor::new(bytes))))
}

struct Pair {
    local: LocalApi,
    http: client::HttpApi,
}

async fn spawn_pair() -> Pair {
    let player = Player::try_with_sink(Box::new(NullSink::new())).expect("headless player starts");
    let provider: FactoryOverride = Arc::new(|_| Some(wav_factory(6)));
    let session = SessionHandle::spawn_with_factory(
        Arc::new(StubLibrary),
        player,
        PlaybackServices::default(),
        provider,
    );
    let token = "contract-token".to_string();
    let state = Arc::new(daemon::http::HttpState {
        api: Arc::new(LocalApi::new(session.clone())),
        session: session.clone(),
        token: token.clone(),
        started: Instant::now(),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(daemon::http::serve(listener, state));
    Pair {
        local: LocalApi::new(session),
        http: client::HttpApi::new(format!("http://{addr}"), token),
    }
}

fn replace(keys: &[&str]) -> SetQueueRequest {
    SetQueueRequest {
        mode: QueueMode::Replace,
        context: QueueContext::Tracks {
            keys: keys.iter().map(|key| (*key).to_string()).collect(),
        },
        start_index: Some(0),
        shuffle: None,
    }
}

async fn wait_state(
    api: &dyn KopuzApi,
    description: &str,
    predicate: impl Fn(&PlayerState) -> bool,
) -> PlayerState {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let state = api.player_state().await.expect("player state");
        if predicate(&state) {
            return state;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {description}: {state:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Wall-clock and anchor fields differ between the two reads by nature;
/// everything else must match bit for bit.
fn normalized(mut state: PlayerState) -> PlayerState {
    state.now_ms = 0;
    state.position = None;
    state
}

#[tokio::test]
async fn reads_agree_between_local_and_http() {
    let pair = spawn_pair().await;
    let ack = pair
        .http
        .set_queue(replace(&["/a.wav", "/b.wav", "/c.wav"]))
        .await
        .expect("set queue over http");
    assert!(ack.rev > 0);
    wait_state(&pair.local, "committed", |state| {
        state.phase == Phase::Playing && matches!(state.intent, Intent::Committed { .. })
    })
    .await;

    let local_state = normalized(pair.local.player_state().await.expect("local state"));
    let http_state = normalized(pair.http.player_state().await.expect("http state"));
    assert_eq!(local_state, http_state);

    let local_window = pair
        .local
        .queue_window(Page::default())
        .await
        .expect("local window");
    let http_window = pair
        .http
        .queue_window(Page::default())
        .await
        .expect("http window");
    assert_eq!(local_window, http_window);
    assert_eq!(http_window.total, 3);
}

#[tokio::test]
async fn commands_and_errors_map_identically() {
    let pair = spawn_pair().await;
    pair.http
        .set_queue(replace(&["/a.wav", "/b.wav"]))
        .await
        .expect("set queue");
    wait_state(&pair.local, "committed", |state| {
        matches!(state.intent, Intent::Committed { .. })
    })
    .await;

    pair.http
        .player_command(PlayerCommand::SetMode {
            shuffle: None,
            loop_mode: Some(LoopMode::Queue),
        })
        .await
        .expect("set mode over http");
    let state = pair.local.player_state().await.expect("state");
    assert_eq!(state.queue.loop_mode, LoopMode::Queue);

    let local_err = pair
        .local
        .queue_edit(QueueEdit::Remove { index: 0 })
        .await
        .expect_err("guarded locally");
    let http_err = pair
        .http
        .queue_edit(QueueEdit::Remove { index: 0 })
        .await
        .expect_err("guarded over http");
    assert_eq!(local_err.code, ErrorCode::InvalidInput);
    assert_eq!(http_err.code, local_err.code);
    assert_eq!(http_err.message, local_err.message);

    let local_err = pair
        .local
        .tracks(TrackFilter::default(), Page::default())
        .await
        .expect_err("unsupported locally");
    let http_err = pair
        .http
        .tracks(TrackFilter::default(), Page::default())
        .await
        .expect_err("unsupported over http");
    assert_eq!(local_err.code, ErrorCode::Unsupported);
    assert_eq!(http_err.code, local_err.code);
}

#[tokio::test]
async fn http_events_stream_delivers_typed_events() {
    use futures_util::StreamExt;
    let pair = spawn_pair().await;
    let mut events = pair.http.events();

    // The stream connects asynchronously and the first subscription has no
    // Last-Event-ID to replay from, so keep nudging until events flow.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut saw_queue_changed = false;
    let mut saw_player_state = false;
    while !(saw_queue_changed && saw_player_state) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for SSE events"
        );
        pair.http
            .player_command(PlayerCommand::SetMode {
                shuffle: Some(true),
                loop_mode: None,
            })
            .await
            .expect("set mode");
        while let Ok(Some(event)) =
            tokio::time::timeout(Duration::from_millis(500), events.next()).await
        {
            match event {
                ApiEvent::QueueChanged { .. } => saw_queue_changed = true,
                ApiEvent::PlayerState(state) if state.queue.shuffle => saw_player_state = true,
                _ => {}
            }
            if saw_queue_changed && saw_player_state {
                break;
            }
        }
    }
}

#[tokio::test]
async fn wrong_token_is_rejected() {
    let pair = spawn_pair().await;
    let base = {
        let probe = pair.http.player_state().await;
        assert!(probe.is_ok(), "control: correct token works");
        pair
    };
    let bad = client::HttpApi::new(base.http_base_for_test(), "wrong-token");
    let err = bad.player_state().await.expect_err("rejected");
    assert_eq!(err.code, ErrorCode::Unauthorized);
}

impl Pair {
    fn http_base_for_test(&self) -> String {
        self.http.base_url().to_string()
    }
}
