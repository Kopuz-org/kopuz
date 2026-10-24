//! Contract tests: the same assertions run through `LocalApi` (in-process)
//! and `GrpcApi` (over a real tonic server and its Subscribe stream), proving
//! the two transports cannot drift. This is the parity mechanism the split
//! relies on.

use std::io::Cursor;
use std::sync::Arc;
use std::time::{Duration, Instant};

use api::{
    ApiError, ApiEvent, ErrorCode, Intent, KopuzApi, LoopMode, Page, Phase, PlayerCommand,
    PlayerState, QueueContext, QueueEdit, QueueMode, SetQueueRequest, TrackFilter, prelude::*,
};
use daemon::session::FactoryOverride;
use daemon::{
    ConfigService, FavoritesService, JobRunner, LibraryService, LocalApi, PlaybackServices,
    QueueMaterializer, SessionHandle,
};
use player::engine::{NullSink, SourceFactory};
use player::player::Player;
use reader::Track;

struct StubLibrary;

#[async_trait::async_trait]
impl QueueMaterializer for StubLibrary {
    async fn materialize(&self, context: &QueueContext) -> Result<Vec<Track>, ApiError> {
        match context {
            // Keys containing "nope" stay unresolved, standing in for a track
            // that neither the DB, the transient cache, nor disk can produce;
            // the favorites contract test uses one to assert NotFound mapping.
            QueueContext::Tracks { keys } => Ok(keys
                .iter()
                .filter(|key| !key.contains("nope"))
                .map(|key| track(key))
                .collect()),
            // An id that says it is unknown or unsaved resolves to nothing, as a source would refuse it.
            QueueContext::Album { id } | QueueContext::Playlist { id }
                if id.contains("unknown") || id.contains("unsaved") =>
            {
                Err(ApiError::not_found("the stub holds no such container"))
            }
            // Any other context resolves to two tracks named after its album, playlist or station.
            QueueContext::Album { id } | QueueContext::Playlist { id } => Ok(vec![
                track(&format!("/lib/{id}-0")),
                track(&format!("/lib/{id}-1")),
            ]),
            QueueContext::Radio { station_id, .. } => Ok(vec![
                track(&format!("/lib/{station_id}-0")),
                track(&format!("/lib/{station_id}-1")),
            ]),
            _ => Err(ApiError::unsupported("stub resolves raw tracks only")),
        }
    }
}

fn track(key: &str) -> Track {
    if key.starts_with("ytm-song:") {
        return music_video_pair(key);
    }
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
        replay_gain: config::ReplayGainInfo::default(),
        counterpart: None,
        credits: vec![],
        explicit: false,
        plays: None,
    }
}

/// A song paired with its music video, which opens on a second of intro.
fn music_video_pair(key: &str) -> Track {
    let mut song = track("/stand-in.wav");
    song.id = reader::models::TrackId::Server {
        service: config::MusicService::YtMusic,
        item_id: key.to_string(),
    };
    song.title = key.to_string();
    song.counterpart = Some(Box::new(reader::Counterpart {
        item_id: format!("{key}-video"),
        video: true,
        duration_ms: Some(7_000),
        segments: vec![reader::SharedSegment {
            start_ms: 0,
            counterpart_start_ms: 1_000,
            duration_ms: 6_000,
        }],
    }));
    song
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
    wire: client::GrpcApi,
    tcp: client::GrpcApi,
    tcp_address: String,
    jobs: Arc<JobRunner>,
    config_service: Arc<ConfigService>,
    library: Arc<LibraryService>,
    favorites: Arc<FavoritesService>,
    playlists: Arc<daemon::PlaylistService>,
    sources: Arc<daemon::SourceService>,
    database: db::Db,
    session: SessionHandle,
    _dir: tempfile::TempDir,
}

async fn spawn_pair() -> Pair {
    let dir = tempfile::tempdir().expect("tempdir");
    let database = db::init(&dir.path().join("contract.db"))
        .await
        .expect("db init");
    let seeded: Vec<Track> = ["/lib/seed-0.flac", "/lib/seed-1.flac"]
        .iter()
        .map(|key| track(key))
        .collect();
    database
        .upsert_tracks(&config::Source::default(), &seeded)
        .await
        .expect("seed tracks");
    // No test may ask the real internet whether it is there; one that needs a check names its own address.
    let quiet = config::Settings {
        system: config::System {
            connectivity_check_urls: Vec::new(),
            ..config::System::default()
        },
        ..config::Settings::default()
    };
    let config_service = Arc::new(ConfigService::new(
        database.clone(),
        dir.path().join("settings.toml"),
        quiet,
    ));
    let credentials = daemon::CredentialStore::load(&database)
        .await
        .expect("credentials");
    let library = Arc::new(LibraryService::new(
        database.clone(),
        config::Source::default(),
        Arc::new(radio::registry::StationRegistry::default()),
        dir.path().join("covers"),
        credentials.clone(),
    ));
    let player = Player::try_with_sink(Box::new(NullSink::new())).expect("headless player starts");
    let provider: FactoryOverride = Arc::new(|_| Some(wav_factory(6)));
    let session = SessionHandle::spawn_with_factory(
        Arc::new(StubLibrary),
        player,
        PlaybackServices::default(),
        provider,
    );
    library.attach_session(session.clone());
    config_service.attach_session(session.clone());
    let jobs = Arc::new(JobRunner::new(session.clone()));
    let favorites = FavoritesService::new(database.clone(), session.clone());
    let artwork = daemon::ArtworkService::new(
        database.clone(),
        session.clone(),
        dir.path().join("artwork"),
    );
    let playlists = daemon::PlaylistService::new(database.clone(), session.clone());
    let mutations = daemon::MutationService::new(
        database.clone(),
        session.clone(),
        dir.path().join("uploads"),
    );
    let downloads = daemon::DownloadsService::new(
        database.clone(),
        session.clone(),
        dir.path().join("offline"),
    );
    let sources = daemon::SourceService::new(
        database.clone(),
        session.clone(),
        config_service.clone(),
        session.sources(),
    );
    sources.watch_active(session.sources_watch());
    let presence_pause = daemon::PresencePause::load(&database)
        .await
        .expect("presence pause");
    let integrations =
        daemon::IntegrationService::new(config_service.clone(), credentials, presence_pause);
    let downloader =
        daemon::UrlDownloadService::new(database.clone(), config_service.clone(), sources.clone());
    let prefs = daemon::PrefsService::new(database.clone(), session.clone());
    let catalog = daemon::CatalogService::new(database.clone(), session.clone(), library.clone());
    let build_api = |session: SessionHandle| {
        LocalApi::new(session)
            .with_config(config_service.clone())
            .with_catalog(catalog.clone())
            .with_prefs(prefs.clone())
            .with_library(library.clone())
            .with_jobs(jobs.clone())
            .with_favorites(favorites.clone())
            .with_artwork(artwork.clone())
            .with_playlists(playlists.clone())
            .with_mutations(mutations.clone())
            .with_sources(sources.clone())
            .with_integrations(integrations.clone())
            .with_downloads(downloads.clone())
            .with_downloader(downloader.clone())
    };
    let state = Arc::new(kopuzd::GrpcState {
        api: Arc::new(build_api(session.clone())),
        artwork: Some(artwork.clone()),
        session: session.clone(),
        started: Instant::now(),
    });
    let socket = dir.path().join("kopuzd.sock");
    let listener = kopuzd::bind_socket(&socket).expect("bind socket");
    tokio::spawn(kopuzd::serve(listener, state.clone()));
    let tcp = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let tcp_address = tcp.local_addr().expect("bound address").to_string();
    tokio::spawn(kopuzd::serve_tcp(tcp, state));
    Pair {
        local: build_api(session.clone()),
        wire: client::GrpcApi::new(&socket).expect("wire client"),
        tcp: client::GrpcApi::connect_tcp(&tcp_address).expect("tcp client"),
        tcp_address,
        jobs,
        config_service,
        library,
        favorites,
        playlists,
        sources,
        database,
        session,
        _dir: dir,
    }
}

async fn clippsly_pair() -> (Pair, String, tokio::task::JoinHandle<()>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let pair = spawn_pair().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let source = pair
        .local
        .upsert_source(api::SourceDraft {
            name: "Test locker".into(),
            service: "clippsly".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    pair.sources
        .mutate(|sources| {
            sources
                .servers
                .iter_mut()
                .find(|row| row.id == source.id)
                .unwrap()
                .url = address;
        })
        .await
        .unwrap();
    pair.database
        .set_server_credentials(&source.id, Some("mock-password"), Some("42"))
        .await
        .unwrap();
    let server = tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let mut data = Vec::new();
            let end = loop {
                let mut buffer = [0; 16384];
                let length = socket.read(&mut buffer).await.unwrap();
                assert!(length > 0);
                data.extend_from_slice(&buffer[..length]);
                if let Some(end) = data.windows(4).position(|part| part == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let headers = String::from_utf8(data[..end].to_vec()).unwrap();
            assert!(
                headers
                    .to_ascii_lowercase()
                    .contains("authorization: bearer mock-password")
            );
            let length = headers
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                .unwrap_or(0);
            while data.len() < end + length {
                let mut buffer = [0; 16384];
                let read = socket.read(&mut buffer).await.unwrap();
                assert!(read > 0);
                data.extend_from_slice(&buffer[..read]);
            }
            let body = if headers.starts_with("GET /v1/account/storage ") {
                r#"{"track_count":1,"used_bytes":10,"quota_bytes":10000000,"quota_remaining_bytes":9999990}"#
            } else if headers.starts_with("POST /v1/upload ") {
                assert!(length > 4 * 1024 * 1024);
                r#"{"success":true}"#
            } else {
                panic!("unexpected mock request: {headers}")
            };
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        }
    });
    (pair, source.id, server)
}

#[tokio::test]
async fn clippsly_quota_and_large_uploads_agree_across_transports() {
    let (pair, id, server) = clippsly_pair().await;
    for api in [&pair.local as &dyn KopuzApi, &pair.wire, &pair.tcp] {
        let source = api
            .sources()
            .await
            .unwrap()
            .into_iter()
            .find(|row| row.id == id)
            .unwrap();
        assert!(source.capabilities.uploads && source.capabilities.storage_quota);
        assert!(source.authenticated);
        assert!(!format!("{source:?}").contains("mock-password"));
        let quota = api.storage_quota(id.clone()).await.unwrap();
        assert_eq!(quota.used_bytes, 10);
        assert_eq!(quota.remaining_bytes, 9_999_990);
        api.upload_track(api::TrackUpload {
            source_id: id.clone(),
            filename: "song.flac".into(),
            content: vec![b'a'; 5 * 1024 * 1024],
        })
        .await
        .unwrap();
        let error = api
            .upload_track(api::TrackUpload {
                source_id: id.clone(),
                filename: "song.flac".into(),
                content: vec![],
            })
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidInput);
        assert_eq!(
            api.storage_quota(config::DEFAULT_LOCAL_ID.into())
                .await
                .unwrap_err()
                .code,
            ErrorCode::Unsupported
        );
    }
    assert!(!server.is_finished(), "the mock server failed");
    server.abort();
}

#[tokio::test]
async fn clippsly_wire_rejects_truncated_oversized_and_changed_upload_metadata() {
    let (pair, id, server) = clippsly_pair().await;
    let mut client =
        proto::kopuz_client::KopuzClient::connect(format!("http://{}", pair.tcp_address))
            .await
            .unwrap();
    let header = proto::TrackUploadChunk {
        source_id: id.clone(),
        filename: "song.flac".into(),
        total_bytes: 4,
        content: Vec::new(),
    };
    for chunks in [
        vec![],
        vec![header.clone()],
        vec![proto::TrackUploadChunk {
            total_bytes: api::MAX_MUSIC_UPLOAD_BYTES as u64 + 1,
            ..header.clone()
        }],
        vec![
            header.clone(),
            proto::TrackUploadChunk {
                content: vec![0; 5],
                ..Default::default()
            },
        ],
        vec![
            header.clone(),
            proto::TrackUploadChunk {
                source_id: id.clone(),
                content: vec![0; 4],
                ..Default::default()
            },
        ],
    ] {
        let request = tonic::Request::new(tokio_stream::iter(chunks));
        let error = client.upload_track(request).await.unwrap_err();
        assert_eq!(error.code(), tonic::Code::InvalidArgument);
    }
    server.abort();
}

#[test]
fn cli_default_address_matches_the_daemon() {
    assert_eq!(client::default_socket_path(), kopuzd::default_socket_path());
}

#[tokio::test]
async fn cli_playback_and_search_work_over_each_transport() {
    let pair = spawn_pair().await;
    for api in [&pair.local as &dyn KopuzApi, &pair.wire, &pair.tcp] {
        let tracks = ctl::execute(
            api,
            ctl::Command::Search {
                query: "seed".into(),
                page: ctl::PageArgs {
                    offset: 1,
                    limit: 1,
                },
            },
            None,
        )
        .await
        .expect("search");
        assert_eq!(tracks.as_array().unwrap().len(), 1);
        let key = tracks[0]["key"].as_str().unwrap().to_string();
        ctl::execute(
            api,
            ctl::Command::Play {
                keys: vec![key.clone()],
            },
            None,
        )
        .await
        .expect("play");
        wait_state(api, "playing", |state| state.phase == Phase::Playing).await;
        ctl::execute(api, ctl::Command::Pause, None)
            .await
            .expect("pause");
        wait_state(api, "paused", |state| state.phase == Phase::Paused).await;
        ctl::execute(api, ctl::Command::Volume { percent: 36 }, None)
            .await
            .expect("volume");
        wait_state(api, "volume applied", |state| {
            (state.volume - 0.36).abs() < f32::EPSILON
        })
        .await;
        let status = ctl::execute(api, ctl::Command::Status, None)
            .await
            .expect("status");
        assert_eq!(status["queue_length"], 1);
        assert_eq!(status["volume_percent"], 36.0);
        let queue = ctl::execute(
            api,
            ctl::Command::Queue(ctl::PageArgs {
                offset: 0,
                limit: 1,
            }),
            None,
        )
        .await
        .expect("queue");
        assert_eq!(queue[0]["key"], key);
        ctl::execute(api, ctl::Command::Stop, None)
            .await
            .expect("stop");
        let overflow = ctl::execute(api, ctl::Command::Seek { seconds: u64::MAX }, None)
            .await
            .expect_err("reject overflowing milliseconds");
        assert_eq!(overflow.code, ErrorCode::InvalidInput);
    }
}

#[tokio::test]
async fn cli_source_credentials_are_write_only_over_each_transport() {
    let pair = spawn_pair().await;
    for api in [&pair.local as &dyn KopuzApi, &pair.wire, &pair.tcp] {
        let added = ctl::execute(
            api,
            ctl::Command::Source {
                command: ctl::SourceCommand::Add {
                    service: "jellyfin".into(),
                    name: "CLI server".into(),
                    url: Some("https://jelly.example".into()),
                    fields: Vec::new(),
                    secret_stdin: None,
                    activate: false,
                },
            },
            None,
        )
        .await
        .expect("add source");
        let id = added["id"].as_str().unwrap().to_string();
        let signed_in = ctl::execute(
            api,
            ctl::Command::Source {
                command: ctl::SourceCommand::Token {
                    id: id.clone(),
                    user_id: Some("alice".into()),
                    secret_stdin: true,
                },
            },
            Some("cli-secret-never-printed".into()),
        )
        .await
        .expect("provision token");
        assert_eq!(signed_in["authenticated"], true);
        assert!(!signed_in.to_string().contains("cli-secret-never-printed"));
        let listed = ctl::execute(
            api,
            ctl::Command::Source {
                command: ctl::SourceCommand::List,
            },
            None,
        )
        .await
        .expect("list sources");
        assert!(!listed.to_string().contains("cli-secret-never-printed"));
        assert!(
            listed
                .as_array()
                .unwrap()
                .iter()
                .any(|source| source["id"] == id)
        );
        ctl::execute(
            api,
            ctl::Command::Source {
                command: ctl::SourceCommand::Logout { id: id.clone() },
            },
            None,
        )
        .await
        .expect("logout");
        assert!(
            !api.sources()
                .await
                .unwrap()
                .iter()
                .find(|source| source.id == id)
                .unwrap()
                .authenticated
        );
        ctl::execute(
            api,
            ctl::Command::Source {
                command: ctl::SourceCommand::Remove { id: id.clone() },
            },
            None,
        )
        .await
        .expect("remove source");
        assert!(
            !api.sources()
                .await
                .unwrap()
                .iter()
                .any(|source| source.id == id)
        );
    }
}

#[tokio::test]
async fn cli_rejects_secret_and_unknown_setup_fields_before_saving() {
    let pair = spawn_pair().await;
    let ids = |sources: Vec<api::SourceInfo>| {
        sources
            .into_iter()
            .map(|source| source.id)
            .collect::<Vec<_>>()
    };
    let before = ids(pair.wire.sources().await.unwrap());
    for (service, key) in [("applemusic", "token"), ("jellyfin", "unknown")] {
        let error = ctl::execute(
            &pair.wire,
            ctl::Command::Source {
                command: ctl::SourceCommand::Add {
                    service: service.into(),
                    name: "Invalid".into(),
                    url: None,
                    fields: vec![api::FieldValue::new(key, "not-for-command-lines")],
                    secret_stdin: None,
                    activate: false,
                },
            },
            None,
        )
        .await
        .expect_err("invalid setup field");
        assert_eq!(error.code, ErrorCode::InvalidInput);
        assert!(!error.to_string().contains("not-for-command-lines"));
    }
    assert_eq!(ids(pair.wire.sources().await.unwrap()), before);
}

async fn panicking_job() -> Result<(), ApiError> {
    panic!("intentional test panic")
}

#[tokio::test]
async fn panicked_jobs_finish_as_failed_and_emit_an_event() {
    let pair = spawn_pair().await;
    let mut events = pair.session.subscribe();
    let job = pair
        .jobs
        .start(api::JobKind::Download, |_| panicking_job())
        .expect("job starts");

    let status = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(status) = pair
                .jobs
                .list()
                .into_iter()
                .find(|status| status.id == job.job_id)
                && status.state != api::JobState::Running
            {
                break status;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("job completion");
    assert_eq!(status.state, api::JobState::Failed);
    assert!(status.error.is_some());

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(ApiEvent::JobFinished { id, ok, error, .. }) = events.recv().await
                && id == job.job_id
            {
                assert!(!ok);
                assert!(error.is_some());
                break;
            }
        }
    })
    .await
    .expect("job-finished event");
}

async fn wait_job(api: &dyn KopuzApi, kind: api::JobKind) -> api::JobStatus {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let jobs = api.jobs().await.expect("jobs");
            if let Some(status) = jobs.into_iter().find(|status| status.kind == kind) {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the job shows up")
}

async fn all_jobs_settled(api: &dyn KopuzApi) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let jobs = api.jobs().await.expect("jobs");
            if jobs.iter().all(|job| job.state != api::JobState::Running) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("jobs settle");
}

#[tokio::test]
async fn a_job_says_whether_the_daemons_schedule_started_it_on_every_transport() {
    use futures_util::StreamExt;
    let pair = spawn_pair().await;
    let mut wire_events = pair.wire.events();

    let user = pair
        .wire
        .start_job(api::JobKind::Scan)
        .await
        .expect("a user's scan starts");
    let scheduled = pair
        .jobs
        .start_as(
            api::JobKind::Download,
            daemon::jobs::Trigger::Schedule,
            |_| async { Ok(()) },
        )
        .expect("a scheduled job starts");
    all_jobs_settled(&pair.local).await;

    let local = pair.local.jobs().await.expect("local jobs");
    assert_eq!(local, pair.wire.jobs().await.expect("wire jobs"));
    assert_eq!(local, pair.tcp.jobs().await.expect("tcp jobs"));
    let flag = |id: &str| {
        local
            .iter()
            .find(|job| job.id == id)
            .map(|job| job.automatic)
    };
    assert_eq!(flag(&user.job_id), Some(false));
    assert_eq!(flag(&scheduled.job_id), Some(true));

    // The stream joins at the live position, so keep starting jobs until its events flow.
    let (mut manual_seen, mut scheduled_seen) = (false, false);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !(manual_seen && scheduled_seen) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for JobFinished events"
        );
        let _ = pair.wire.start_job(api::JobKind::Scan).await;
        let _ = pair.jobs.start_as(
            api::JobKind::Download,
            daemon::jobs::Trigger::Schedule,
            |_| async { Ok(()) },
        );
        while let Ok(Some(event)) =
            tokio::time::timeout(Duration::from_millis(300), wire_events.next()).await
        {
            match event {
                ApiEvent::JobFinished {
                    kind: api::JobKind::Scan,
                    automatic,
                    ..
                } => {
                    assert!(!automatic, "a scan the user asked for");
                    manual_seen = true;
                }
                ApiEvent::JobFinished {
                    kind: api::JobKind::Download,
                    automatic,
                    ..
                } => {
                    assert!(automatic, "a job the schedule started");
                    scheduled_seen = true;
                }
                ApiEvent::JobProgress(progress) => {
                    assert_eq!(progress.automatic, progress.kind == api::JobKind::Download);
                }
                _ => {}
            }
        }
    }
}

#[tokio::test]
async fn starting_a_running_kind_joins_the_job_on_every_transport() {
    let pair = spawn_pair().await;
    let (finish, finished) = tokio::sync::oneshot::channel::<()>();
    let running = pair
        .jobs
        .start_as(
            api::JobKind::Scan,
            daemon::jobs::Trigger::Schedule,
            |_| async move {
                let _ = finished.await;
                Ok(())
            },
        )
        .expect("the first scan starts");

    let local = pair.local.start_job(api::JobKind::Scan).await;
    let wire = pair.wire.start_job(api::JobKind::Scan).await;
    let tcp = pair.tcp.start_job(api::JobKind::Scan).await;
    assert_eq!(local.expect("joined locally"), running);
    assert_eq!(wire.expect("joined on the wire"), running);
    assert_eq!(tcp.expect("joined over tcp"), running);
    let scans = pair
        .local
        .jobs()
        .await
        .expect("jobs")
        .into_iter()
        .filter(|job| job.kind == api::JobKind::Scan)
        .count();
    assert_eq!(scans, 1, "no second scan");

    let _ = finish.send(());
    all_jobs_settled(&pair.local).await;
}

const SYNC_INTERVAL: config::SettingKey =
    config::SettingKey::Library(config::LibraryKey::SyncIntervalMinutes);

async fn activate_jellyfin(pair: &Pair, minutes: u32) {
    pair.config_service
        .mutate_state(&[SYNC_INTERVAL], move |config| {
            config.library.sync_interval_minutes = minutes;
        })
        .await
        .expect("config saved");
    pair.sources
        .mutate(|sources| {
            sources.server = Some(config::MusicServer {
                name: "unreachable".into(),
                url: "http://127.0.0.1:9".into(),
                access_token: Some("token".into()),
                user_id: Some("user".into()),
                id: Some("srv-1".into()),
                ..Default::default()
            });
            sources.active = config::Source::Server("srv-1".into());
        })
        .await
        .expect("sources saved");
}

fn spawn_auto_sync(pair: &Pair) {
    daemon::auto_sync::spawn(
        pair.database.clone(),
        pair.jobs.clone(),
        pair.library.clone(),
        pair.playlists.clone(),
        pair.favorites.clone(),
        pair.session.settings_watch(),
        pair.session.sources_watch(),
    );
}

#[tokio::test]
async fn the_daemons_own_syncs_are_automatic_on_every_transport() {
    let pair = spawn_pair().await;
    activate_jellyfin(&pair, 60).await;
    spawn_auto_sync(&pair);

    for kind in [
        api::JobKind::LibrarySync,
        api::JobKind::PlaylistSync,
        api::JobKind::FavoritesSync,
    ] {
        assert!(wait_job(&pair.local, kind).await.automatic, "{kind:?}");
        assert!(
            wait_job(&pair.wire, kind).await.automatic,
            "{kind:?} on the wire"
        );
    }
}

#[tokio::test]
async fn a_zero_sync_interval_still_syncs_a_source_never_synced() {
    let pair = spawn_pair().await;
    activate_jellyfin(&pair, 0).await;
    spawn_auto_sync(&pair);

    for kind in [
        api::JobKind::LibrarySync,
        api::JobKind::PlaylistSync,
        api::JobKind::FavoritesSync,
    ] {
        assert!(wait_job(&pair.wire, kind).await.automatic, "{kind:?}");
    }
}

#[tokio::test]
async fn a_zero_sync_interval_starts_no_syncs_of_a_synced_source() {
    let pair = spawn_pair().await;
    let source = config::Source::Server("srv-1".into());
    // Synced long ago, so only the interval keeps the syncs from being due.
    for kind in [
        api::JobKind::LibrarySync,
        api::JobKind::PlaylistSync,
        api::JobKind::FavoritesSync,
    ] {
        let key = daemon::auto_sync::stamp_key(kind).expect("a sync kind");
        pair.database
            .meta_put(key, source.as_str(), "0")
            .await
            .expect("stamped");
    }
    activate_jellyfin(&pair, 0).await;
    spawn_auto_sync(&pair);

    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(pair.local.jobs().await.expect("jobs").is_empty());

    // Raising it from 0 is a change the running schedule must notice.
    let report = pair
        .local
        .edit_config(vec![config::SettingsEdit::Library(
            config::LibraryEdit::SyncIntervalMinutes(60),
        )])
        .await
        .expect("interval set");
    assert_eq!(report.outcomes, vec![api::EditOutcome::Applied]);
    assert!(
        wait_job(&pair.wire, api::JobKind::LibrarySync)
            .await
            .automatic,
        "the schedule re-armed"
    );
}

#[tokio::test]
async fn the_daemons_own_folder_scan_is_automatic_and_a_users_is_not() {
    let pair = spawn_pair().await;
    let music = tempfile::tempdir().expect("tempdir");
    pair.config_service
        .mutate_state(
            &[config::SettingKey::Library(
                config::LibraryKey::LocalSources,
            )],
            |config| {
                config.library.local_sources =
                    vec![config::SavedLocalSource::default_library(vec![
                        music.path().to_path_buf(),
                    ])];
            },
        )
        .await
        .expect("config saved");
    daemon::folder_scan::spawn(
        pair.jobs.clone(),
        pair.library.clone(),
        pair.session.settings_watch(),
        pair.session.subscribe(),
        true,
    );
    let scheduled = wait_job(&pair.local, api::JobKind::Scan).await;
    assert!(scheduled.automatic);
    all_jobs_settled(&pair.local).await;

    let user = pair
        .wire
        .start_job(api::JobKind::Scan)
        .await
        .expect("a user's scan starts");
    let jobs = pair.wire.jobs().await.expect("wire jobs");
    let started = jobs
        .iter()
        .find(|job| job.id == user.job_id)
        .expect("the user's scan is listed");
    assert!(!started.automatic);
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
async fn reads_agree_between_local_and_wire() {
    let pair = spawn_pair().await;
    let ack = pair
        .wire
        .set_queue(replace(&["/a.wav", "/b.wav", "/c.wav"]))
        .await
        .expect("set queue over http");
    assert!(ack.rev > 0);
    wait_state(&pair.local, "committed", |state| {
        state.phase == Phase::Playing && matches!(state.intent, Intent::Committed { .. })
    })
    .await;

    let local_state = normalized(pair.local.player_state().await.expect("local state"));
    let wire_state = normalized(pair.wire.player_state().await.expect("wire state"));
    assert_eq!(local_state, wire_state);

    let local_window = pair
        .local
        .queue_window(Page::default())
        .await
        .expect("local window");
    let wire_window = pair
        .wire
        .queue_window(Page::default())
        .await
        .expect("wire window");
    assert_eq!(local_window, wire_window);
    assert_eq!(wire_window.total, 3);
}

#[tokio::test]
async fn commands_and_errors_map_identically() {
    let pair = spawn_pair().await;
    pair.wire
        .set_queue(replace(&["/a.wav", "/b.wav"]))
        .await
        .expect("set queue");
    wait_state(&pair.local, "committed", |state| {
        matches!(state.intent, Intent::Committed { .. })
    })
    .await;

    pair.wire
        .player_command(PlayerCommand::SetMode {
            shuffle: None,
            loop_mode: Some(LoopMode::Queue),
        })
        .await
        .expect("set mode over the wire");
    let state = pair.local.player_state().await.expect("state");
    assert_eq!(state.queue.loop_mode, LoopMode::Queue);

    let local_err = pair
        .local
        .queue_edit(QueueEdit::Remove { index: 0 })
        .await
        .expect_err("guarded locally");
    let wire_err = pair
        .wire
        .queue_edit(QueueEdit::Remove { index: 0 })
        .await
        .expect_err("guarded over http");
    assert_eq!(local_err.code, ErrorCode::InvalidInput);
    assert_eq!(wire_err.code, local_err.code);
    assert_eq!(wire_err.message, local_err.message);

    let local_page = pair
        .local
        .tracks(TrackFilter::default(), Page::default())
        .await
        .expect("tracks locally");
    let wire_page = pair
        .wire
        .tracks(TrackFilter::default(), Page::default())
        .await
        .expect("tracks over the wire");
    assert_eq!(local_page, wire_page);
    assert_eq!(wire_page.total, 2);
}

#[tokio::test]
async fn mute_round_trips_across_transports_and_keeps_the_volume() {
    let pair = spawn_pair().await;
    pair.wire
        .player_command(PlayerCommand::SetVolume { volume: 0.6 })
        .await
        .expect("set volume over the wire");
    pair.wire
        .player_command(PlayerCommand::SetMuted { muted: true })
        .await
        .expect("mute over the wire");
    let local = normalized(pair.local.player_state().await.expect("local state"));
    let wire = normalized(pair.wire.player_state().await.expect("wire state"));
    assert!(wire.muted);
    assert_eq!(wire.volume, 0.6);
    assert_eq!(local, wire);

    pair.local
        .player_command(PlayerCommand::SetMuted { muted: false })
        .await
        .expect("unmute locally");
    let wire = pair.wire.player_state().await.expect("wire state");
    assert!(!wire.muted);
    assert_eq!(wire.volume, 0.6);
}

#[tokio::test]
async fn subscribe_stream_delivers_typed_events() {
    use futures_util::StreamExt;
    let pair = spawn_pair().await;
    let mut events = pair.wire.events();

    // The stream connects asynchronously and the first subscription starts
    // at the current live position, so keep nudging until events flow.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut saw_queue_changed = false;
    let mut saw_player_state = false;
    while !(saw_queue_changed && saw_player_state) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for Subscribe events"
        );
        pair.wire
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

fn active_state(rows: Vec<api::SourceInfo>) -> Option<api::SourceState> {
    rows.into_iter()
        .find(|row| row.active)
        .and_then(|row| row.state)
}

/// Wait out the daemon's first probe of its active source, which runs in the background from boot.
async fn wait_probed(pair: &Pair) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let state = active_state(pair.local.sources().await.expect("local sources"));
        if state.is_some_and(|state| state != api::SourceState::Checking) {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the daemon never probed its active source: {state:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// The connection dot reads the daemon's probe off the source row, the same on both transports.
#[tokio::test]
async fn the_active_sources_status_is_the_daemons_probe() {
    let pair = spawn_pair().await;
    wait_probed(&pair).await;
    let local = active_state(pair.local.sources().await.expect("local sources"));
    let wire = active_state(pair.wire.sources().await.expect("wire sources"));
    assert_eq!(
        local,
        Some(api::SourceState::Online),
        "a folder source is always reachable"
    );
    assert_eq!(wire, local, "the wire reports what the daemon holds");
}

/// Every edit variant crosses the wire, so the same batch earns the same outcomes and leaves the same view on either transport.
#[tokio::test]
async fn config_view_and_edits_agree_across_transports() {
    let pair = spawn_pair().await;

    let local_view = pair.local.config().await.expect("local view");
    let wire_view = pair.wire.config().await.expect("wire view");
    // The whole config surface has to survive the proto round trip for
    // these to be equal, so this is the guard on every field mapping.
    assert_eq!(local_view, wire_view);

    let edits = config::edit_samples();
    let wire = pair
        .wire
        .edit_config(edits.clone())
        .await
        .expect("wire edit");
    let local = pair
        .local
        .edit_config(edits.clone())
        .await
        .expect("local edit");
    assert_eq!(wire.outcomes.len(), edits.len());
    assert_eq!(local.outcomes, wire.outcomes);
    for (edit, outcome) in edits.iter().zip(&wire.outcomes) {
        let owned = config::SettingKey::of(edit).daemon_owned();
        assert_eq!(
            matches!(outcome, api::EditOutcome::Invalid { .. }),
            owned,
            "{edit:?}: {outcome:?}"
        );
    }
    assert_eq!(
        local.view, wire.view,
        "a no-op repeat leaves the view as is"
    );
    assert_eq!(pair.wire.config().await.expect("wire view"), local.view);
    assert_eq!(pair.tcp.config().await.expect("tcp view"), local.view);
}

#[tokio::test]
async fn edit_config_touches_only_named_keys_across_transports() {
    let pair = spawn_pair().await;
    let mut events = pair.session.subscribe();
    let stale = pair.wire.config().await.expect("wire view");

    let other = pair
        .local
        .edit_config(vec![config::SettingsEdit::Playback(
            config::PlaybackEdit::CrossfadeSeconds(7),
        )])
        .await
        .expect("local edit");
    assert_eq!(other.view.settings.playback.crossfade_seconds, 7);
    assert!(other.view.revision > stale.revision);

    // The wire client never saw the crossfade change; its edits must not undo it.
    let written = pair
        .wire
        .edit_config(vec![
            config::SettingsEdit::Ui(config::UiEdit::Theme("nord".into())),
            config::SettingsEdit::Library(config::LibraryEdit::OfflineQuality(
                config::OfflineQuality::Kbps160,
            )),
        ])
        .await
        .expect("wire edit")
        .view;
    assert_eq!(written.settings.ui.theme, "nord");
    assert_eq!(written.settings.playback.crossfade_seconds, 7);
    assert_eq!(
        written.settings.library.offline_quality,
        config::OfflineQuality::Kbps160
    );
    assert_eq!(
        written.revision,
        other.view.revision + 1,
        "one batch, one save"
    );
    assert_eq!(pair.local.config().await.expect("local view"), written);
    assert_eq!(pair.tcp.config().await.expect("tcp view"), written);

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(ApiEvent::ConfigChanged { revision }) = events.recv().await
                && revision == written.revision
            {
                break;
            }
        }
    })
    .await
    .expect("config-changed event carries the view revision");

    let refused = vec![
        config::SettingsEdit::Playback(config::PlaybackEdit::CrossfadeSeconds(13)),
        config::SettingsEdit::Lyrics(config::LyricsEdit::LyricsDepthBlurStrength(5)),
        config::SettingsEdit::Spotify(config::SpotifyEdit::SpotifyBrowser(Some("x".into()))),
        config::SettingsEdit::Downloads(config::DownloadsEdit::DownloaderOutputDir("/x".into())),
    ];
    let local = pair
        .local
        .edit_config(refused.clone())
        .await
        .expect("local answers");
    let wire = pair.wire.edit_config(refused).await.expect("wire answers");
    assert_eq!(local.outcomes, wire.outcomes);
    assert!(
        local
            .outcomes
            .iter()
            .all(|outcome| matches!(outcome, api::EditOutcome::Invalid { .. })),
        "{:?}",
        local.outcomes
    );
    assert_eq!(local.view, written, "refused edits change nothing");
    assert_eq!(wire.view, written);
}

fn pref(key: &str, value: Option<&str>) -> api::PrefEntry {
    api::PrefEntry {
        key: key.into(),
        value: value.map(Into::into),
    }
}

fn stored(key: &str, value: &str) -> api::FrontendPref {
    api::FrontendPref {
        key: key.into(),
        value: value.into(),
    }
}

async fn next_prefs_event(
    events: &mut tokio::sync::broadcast::Receiver<ApiEvent>,
) -> (String, Vec<String>) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(ApiEvent::FrontendPrefsChanged { frontend, keys }) = events.recv().await {
                break (frontend, keys);
            }
        }
    })
    .await
    .expect("prefs-changed event")
}

#[tokio::test]
async fn frontend_prefs_are_namespaced_and_agree_across_transports() {
    let pair = spawn_pair().await;
    let mut events = pair.session.subscribe();
    assert!(
        pair.wire
            .frontend_prefs("gpui".into())
            .await
            .unwrap()
            .is_empty()
    );

    pair.wire
        .set_frontend_prefs(
            "gpui".into(),
            vec![
                pref("skin", Some("dark")),
                pref("layout", Some("{\"cols\": [1, 2]}")),
                pref("blank", Some("")),
            ],
        )
        .await
        .expect("wire write");
    assert_eq!(
        next_prefs_event(&mut events).await,
        (
            "gpui".to_string(),
            vec![
                "skin".to_string(),
                "layout".to_string(),
                "blank".to_string()
            ]
        )
    );

    pair.local
        .set_frontend_prefs("dioxus".into(), vec![pref("skin", Some("light"))])
        .await
        .expect("local write");
    assert_eq!(
        next_prefs_event(&mut events).await,
        ("dioxus".to_string(), vec!["skin".to_string()])
    );

    let expected = vec![
        stored("blank", ""),
        stored("layout", "{\"cols\": [1, 2]}"),
        stored("skin", "dark"),
    ];
    assert_eq!(
        pair.local.frontend_prefs("gpui".into()).await.unwrap(),
        expected
    );
    assert_eq!(
        pair.wire.frontend_prefs("gpui".into()).await.unwrap(),
        expected
    );
    assert_eq!(
        pair.tcp.frontend_prefs("gpui".into()).await.unwrap(),
        expected
    );
    assert_eq!(
        pair.wire.frontend_prefs("dioxus".into()).await.unwrap(),
        vec![stored("skin", "light")]
    );

    // A delete and a rewrite of an unchanged value: only the delete changed anything.
    pair.wire
        .set_frontend_prefs(
            "gpui".into(),
            vec![
                pref("blank", None),
                pref("skin", Some("dark")),
                pref("never", None),
            ],
        )
        .await
        .expect("delete over the wire");
    assert_eq!(
        next_prefs_event(&mut events).await,
        ("gpui".to_string(), vec!["blank".to_string()])
    );
    assert_eq!(
        pair.local.frontend_prefs("gpui".into()).await.unwrap(),
        vec![
            stored("layout", "{\"cols\": [1, 2]}"),
            stored("skin", "dark")
        ]
    );

    // A write that changes nothing emits nothing.
    pair.local
        .set_frontend_prefs("gpui".into(), vec![pref("skin", Some("dark"))])
        .await
        .expect("no-op write");
    pair.local
        .set_frontend_prefs("gpui".into(), vec![pref("skin", Some("darker"))])
        .await
        .expect("real write");
    assert_eq!(
        next_prefs_event(&mut events).await,
        ("gpui".to_string(), vec!["skin".to_string()]),
        "the no-op write was silent"
    );
}

#[tokio::test]
async fn frontend_prefs_refuse_bad_input_identically_and_store_nothing() {
    let pair = spawn_pair().await;
    let mut events = pair.session.subscribe();
    let too_long = "k".repeat(api::MAX_PREF_KEY_BYTES + 1);
    let huge = "v".repeat(api::MAX_PREF_VALUE_BYTES + 1);
    let cases: Vec<(String, Vec<api::PrefEntry>)> = vec![
        (String::new(), vec![pref("k", Some("v"))]),
        ("f".repeat(api::MAX_PREF_FRONTEND_BYTES + 1), vec![]),
        ("gpui".into(), vec![pref("", Some("v"))]),
        ("gpui".into(), vec![pref(&too_long, Some("v"))]),
        ("gpui".into(), vec![pref("big", Some(&huge))]),
        ("gpui".into(), vec![pref("k", Some("1")), pref("k", None)]),
        // One bad entry refuses the good one before it.
        (
            "gpui".into(),
            vec![pref("good", Some("v")), pref("", Some("v"))],
        ),
    ];
    for (frontend, entries) in cases {
        let local = pair
            .local
            .set_frontend_prefs(frontend.clone(), entries.clone())
            .await
            .expect_err("local refuses");
        let wire = pair
            .wire
            .set_frontend_prefs(frontend, entries)
            .await
            .expect_err("wire refuses");
        assert_eq!(local.code, ErrorCode::InvalidInput);
        assert_eq!(wire.code, local.code);
    }
    for api in [&pair.local as &dyn KopuzApi, &pair.wire] {
        assert_eq!(
            api.frontend_prefs(String::new()).await.unwrap_err().code,
            ErrorCode::InvalidInput
        );
        assert!(api.frontend_prefs("gpui".into()).await.unwrap().is_empty());
    }

    // Nothing was announced for any refused write.
    pair.local
        .set_frontend_prefs("gpui".into(), vec![pref("after", Some("v"))])
        .await
        .expect("a valid write");
    assert_eq!(
        next_prefs_event(&mut events).await,
        ("gpui".to_string(), vec!["after".to_string()])
    );
}

#[tokio::test]
async fn playback_settings_and_their_capabilities_agree_across_transports() {
    let pair = spawn_pair().await;

    let before = pair.wire.config().await.expect("wire view").settings;
    assert_eq!(before.playback.stream_quality, config::StreamQuality::High);
    assert!(
        !before.playback.autoplay_radio
            && !before.playback.skip_explicit
            && !before.playback.pause_watch_history
    );
    let report = pair
        .wire
        .edit_config(vec![
            config::SettingsEdit::Playback(config::PlaybackEdit::StreamQuality(
                config::StreamQuality::Low,
            )),
            config::SettingsEdit::Playback(config::PlaybackEdit::AutoplayRadio(true)),
            config::SettingsEdit::Playback(config::PlaybackEdit::SkipExplicit(true)),
            config::SettingsEdit::Playback(config::PlaybackEdit::PauseWatchHistory(true)),
        ])
        .await
        .expect("edit over the wire");
    assert!(
        report
            .outcomes
            .iter()
            .all(|outcome| *outcome == api::EditOutcome::Applied)
    );
    let written = report.view;
    assert_eq!(
        written.settings.playback.stream_quality,
        config::StreamQuality::Low
    );
    assert!(written.settings.playback.autoplay_radio && written.settings.playback.skip_explicit);
    assert!(written.settings.playback.pause_watch_history);
    let local = pair.local.config().await.expect("local view after set");
    assert_eq!(local.settings, written.settings);

    let active = |rows: Vec<api::SourceInfo>| {
        rows.into_iter()
            .find(|source| source.active)
            .expect("an active source")
            .capabilities
    };
    let local = active(pair.local.sources().await.expect("local sources"));
    let wire = active(pair.wire.sources().await.expect("wire sources"));
    assert_eq!(local, wire);
    assert!(!wire.stream_quality && !wire.explicit_flags && !wire.watch_history);
    assert!(
        !wire.track_radio,
        "a folder library has no radio to autoplay"
    );
}

#[tokio::test]
async fn favorites_round_trip_across_transports() {
    let pair = spawn_pair().await;
    pair.wire
        .set_favorite("/lib/seed-0.flac".into(), true)
        .await
        .expect("set over the wire");
    let local_view = pair.local.favorites().await.expect("local list");
    let wire_view = pair.wire.favorites().await.expect("wire list");
    assert_eq!(local_view.refs, wire_view.refs);
    assert!(local_view.refs.contains(&"/lib/seed-0.flac".to_string()));

    pair.local
        .set_favorite("/lib/seed-0.flac".into(), false)
        .await
        .expect("unset locally");
    let wire_view = pair.wire.favorites().await.expect("wire list");
    assert!(wire_view.refs.is_empty());

    let err = pair
        .wire
        .set_favorite("/nope.flac".into(), true)
        .await
        .expect_err("unknown key");
    assert_eq!(err.code, ErrorCode::NotFound);
}

/// A local library has no recommender to signal, so the capability is off and
/// the op refuses. The refusal is the contract: it has to arrive as
/// `Unsupported` over the wire too, not as a transport failure, because that
/// is what tells a client the button should never have been drawn.
#[tokio::test]
async fn dont_recommend_is_refused_by_a_source_without_it() {
    let pair = spawn_pair().await;
    let local = pair
        .local
        .dont_recommend("/lib/seed-0.flac".into())
        .await
        .expect_err("local source has no recommender");
    let wire = pair
        .wire
        .dont_recommend("/lib/seed-0.flac".into())
        .await
        .expect_err("same over the wire");
    assert_eq!(local.code, ErrorCode::Unsupported);
    assert_eq!(wire.code, ErrorCode::Unsupported);

    let caps = pair
        .wire
        .sources()
        .await
        .expect("sources")
        .into_iter()
        .find(|source| source.active)
        .expect("an active source")
        .capabilities;
    assert!(!caps.dont_recommend);
}

/// Both calls answer `Unsupported` on both transports, not a transport error.
#[tokio::test]
async fn accounts_are_refused_by_a_source_without_them() {
    let pair = spawn_pair().await;
    let draft = api::SourceDraft {
        name: "Home".into(),
        service: "jellyfin".into(),
        values: vec![api::FieldValue::new("url", "https://jelly.example")],
        ..Default::default()
    };
    let server = pair.wire.upsert_source(draft).await.expect("add server");
    let local_id = pair
        .wire
        .sources()
        .await
        .expect("sources")
        .into_iter()
        .find(|source| source.active)
        .expect("an active source")
        .id;

    for id in [local_id, server.id.clone()] {
        let listed = [
            pair.local.accounts(id.clone()).await.expect_err("local"),
            pair.wire.accounts(id.clone()).await.expect_err("wire"),
        ];
        let switched = [
            pair.local
                .switch_account(id.clone(), Some("brand".into()))
                .await
                .expect_err("local"),
            pair.wire
                .switch_account(id.clone(), None)
                .await
                .expect_err("wire"),
        ];
        for error in listed.iter().chain(&switched) {
            assert_eq!(error.code, ErrorCode::Unsupported, "{id}: {error:?}");
        }
    }

    let sources = pair.wire.sources().await.expect("sources");
    assert!(sources.iter().all(|source| !source.capabilities.accounts));
    assert_eq!(sources, pair.local.sources().await.expect("local sources"));

    let missing = [
        pair.local.accounts("nope".into()).await.expect_err("local"),
        pair.wire.accounts("nope".into()).await.expect_err("wire"),
    ];
    assert_eq!(missing[0].code, missing[1].code);
}

/// Switching an inactive source touches no network and no playback.
#[tokio::test]
async fn an_account_choice_is_kept_with_its_source() {
    let pair = spawn_pair().await;
    let draft = api::SourceDraft {
        name: "Music".into(),
        service: "ytmusic".into(),
        values: vec![api::FieldValue::new("auth_method", "browser")],
        ..Default::default()
    };
    let added = pair.wire.upsert_source(draft).await.expect("add source");
    assert!(!added.capabilities.accounts, "no sign-in, no accounts");
    pair.local
        .provision_credentials(api::CredentialProvision {
            server_id: added.id.clone(),
            secret: "SAPISID=not-a-real-cookie".into(),
            user_id: Some("yt-test".into()),
            browser: None,
        })
        .await
        .expect("provision");

    let switched = pair
        .wire
        .switch_account(added.id.clone(), Some("brand-1".into()))
        .await
        .expect("switch over the wire");
    assert!(switched.capabilities.accounts);
    assert!(
        !switched.active,
        "switching an account does not switch sources"
    );
    let stored = |pair: &Pair, id: String| {
        let database = pair.database.clone();
        async move { database.load_server(&id).await.unwrap().unwrap().account }
    };
    assert_eq!(
        stored(&pair, added.id.clone()).await.as_deref(),
        Some("brand-1")
    );

    let back = pair
        .local
        .switch_account(added.id.clone(), None)
        .await
        .expect("switch back locally");
    assert_eq!(back, switched, "the row a client renders does not change");
    assert_eq!(stored(&pair, added.id.clone()).await, None);
}

#[tokio::test]
async fn library_actions_are_refused_by_a_source_without_them() {
    let pair = spawn_pair().await;
    for side in [&pair.local as &dyn KopuzApi, &pair.wire] {
        let refusals = [
            side.rate("/lib/seed-0.flac".into(), api::Rating::Dislike)
                .await
                .expect_err("no rating"),
            side.follow("artist".into(), true)
                .await
                .expect_err("no following"),
            side.save("/lib/seed-0.flac".into(), true)
                .await
                .expect_err("no saving"),
            side.remove_from_history("token".into())
                .await
                .expect_err("no history"),
        ];
        for refusal in refusals {
            assert_eq!(refusal.code, ErrorCode::Unsupported, "{refusal:?}");
        }
    }

    let caps = pair
        .wire
        .sources()
        .await
        .expect("sources")
        .into_iter()
        .find(|source| source.active)
        .expect("an active source")
        .capabilities;
    assert!(!caps.rate && !caps.follow && !caps.save);
    assert!(!caps.remove_from_history && !caps.playlist_details);
}

/// A description the source cannot keep is refused whole, not half-applied as a rename.
#[tokio::test]
async fn a_playlist_edit_needs_details_to_push_them() {
    let pair = spawn_pair().await;
    let id = pair
        .wire
        .create_playlist("Edited".into(), Vec::new())
        .await
        .expect("create");

    pair.wire
        .edit_playlist(
            id.clone(),
            api::PlaylistEdit {
                name: Some("Edited over the wire".into()),
                ..Default::default()
            },
        )
        .await
        .expect("a name alone is a rename");
    for side in [&pair.local as &dyn KopuzApi, &pair.wire] {
        let refused = side
            .edit_playlist(
                id.clone(),
                api::PlaylistEdit {
                    name: Some("Never applied".into()),
                    description: Some("notes".into()),
                    privacy: Some(api::PlaylistPrivacy::Private),
                },
            )
            .await
            .expect_err("a local playlist has no details");
        assert_eq!(refused.code, ErrorCode::Unsupported);
    }

    let catalog = pair.local.playlists().await.expect("catalog");
    let playlist = catalog
        .playlists
        .iter()
        .find(|playlist| playlist.id == id)
        .expect("still there");
    assert_eq!(playlist.name, "Edited over the wire");
}

/// Scan the library and wait for the job to finish.
async fn run_scan(pair: &Pair) {
    let job = pair
        .wire
        .start_job(api::JobKind::Scan)
        .await
        .expect("start scan");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let jobs = pair.local.jobs().await.expect("jobs");
        let status = jobs
            .iter()
            .find(|status| status.id == job.job_id)
            .expect("job listed");
        match status.state {
            api::JobState::Running => {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "scan timed out: {status:?}"
                );
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            api::JobState::Finished => return,
            other => panic!("scan ended as {other:?}: {status:?}"),
        }
    }
}

/// The library's artist names and album titles, sorted.
async fn shelves(api: &dyn KopuzApi) -> (Vec<String>, Vec<String>) {
    let mut artists: Vec<String> = api
        .artists(api::ArtistQuery::default(), Page::default())
        .await
        .expect("artists")
        .artists
        .into_iter()
        .map(|artist| artist.name)
        .collect();
    let mut albums: Vec<String> = api
        .albums(api::AlbumQuery::default(), Page::default())
        .await
        .expect("albums")
        .albums
        .into_iter()
        .map(|album| album.title)
        .collect();
    artists.sort();
    albums.sort();
    (artists, albums)
}

/// A tag edit files what a rescan of the file would, splits no names, and leaves no emptied album or artist behind.
#[tokio::test]
async fn a_tag_edit_files_what_a_rescan_would() {
    let pair = spawn_pair().await;
    let music = pair._dir.path().join("music");
    std::fs::create_dir_all(&music).expect("music dir");
    std::fs::write(music.join("one.wav"), wav_bytes(1)).expect("write wav");
    pair.wire
        .set_source_settings(
            "local".into(),
            vec![api::FieldValue::new(
                "directories",
                api::encode_directories(&[music.to_string_lossy().into_owned()]),
            )],
        )
        .await
        .expect("point the library at the temp dir");
    run_scan(&pair).await;
    let key = pair
        .wire
        .tracks(TrackFilter::default(), Page::default())
        .await
        .expect("tracks")
        .items
        .into_iter()
        .find(|track| track.title.contains("one"))
        .expect("the scanned file")
        .key;

    pair.wire
        .update_track_metadata(api::TrackMetadataPatch {
            key,
            artist: Some("Ada, Bob".into()),
            album: Some("Edited".into()),
            ..Default::default()
        })
        .await
        .expect("edit the tags");

    let edited = shelves(&pair.local).await;
    assert_eq!(
        edited,
        (vec!["Ada, Bob".to_string()], vec!["Edited".to_string()]),
        "one artist, no split, and the emptied album and its artist are gone"
    );
    assert_eq!(shelves(&pair.wire).await, edited);
    run_scan(&pair).await;
    assert_eq!(
        shelves(&pair.local).await,
        edited,
        "a rescan of the edited file files the same"
    );
}

#[tokio::test]
async fn scan_job_indexes_local_files_over_the_wire() {
    let pair = spawn_pair().await;
    let music = pair._dir.path().join("music");
    std::fs::create_dir_all(&music).expect("music dir");
    std::fs::write(music.join("one.wav"), wav_bytes(1)).expect("write wav");
    std::fs::write(music.join("two.wav"), wav_bytes(1)).expect("write wav");

    pair.wire
        .set_source_settings(
            "local".into(),
            vec![api::FieldValue::new(
                "directories",
                api::encode_directories(&[music.to_string_lossy().into_owned()]),
            )],
        )
        .await
        .expect("point the library at the temp dir");
    run_scan(&pair).await;

    let page = pair
        .wire
        .tracks(TrackFilter::default(), Page::default())
        .await
        .expect("tracks over the wire");
    assert!(
        page.items
            .iter()
            .filter(|track| track.title.contains("one") || track.title.contains("two"))
            .count()
            >= 2,
        "scanned tracks visible: {:?}",
        page.items
            .iter()
            .map(|t| t.title.clone())
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn folders_and_stats_agree_across_transports() {
    let pair = spawn_pair().await;
    let local_page = pair
        .local
        .folder_tracks("/lib/".into(), Page::default())
        .await
        .expect("folders locally");
    let wire_page = pair
        .wire
        .folder_tracks("/lib/".into(), Page::default())
        .await
        .expect("folders over the wire");
    assert_eq!(local_page, wire_page);
    assert_eq!(wire_page.total, 2);
    let row = &wire_page.items[0];
    assert_eq!(row.key, "/lib/seed-0.flac");
    assert!(!row.offline);

    let local_stats = pair.local.stats().await.expect("stats locally");
    let wire_stats = pair.wire.stats().await.expect("stats over the wire");
    assert_eq!(local_stats, wire_stats);
}

/// The TCP transport is the same API with no credentials: a client that
/// presents nothing is served, and reads agree with the socket.
#[tokio::test]
async fn the_tcp_transport_serves_a_client_without_a_token() {
    let pair = spawn_pair().await;

    let over_socket = pair.wire.player_state().await.expect("socket");
    let over_tcp = pair.tcp.player_state().await.expect("tcp without a token");
    assert_eq!(over_socket, over_tcp);
}

/// The other half of the UNAVAILABLE split: tonic raises that code itself
/// when it cannot reach the socket, and the daemon sends it for a media
/// server that did not answer. A frontend has to tell those apart to know
/// whether to show "kopuzd is not running" or "your server is down".
#[tokio::test]
async fn a_missing_daemon_is_not_reported_as_a_dead_media_server() {
    let dir = tempfile::tempdir().expect("tempdir");
    let never_bound = dir.path().join("kopuzd.sock");
    let api = client::GrpcApi::new(&never_bound).expect("client builds");

    let error = api.player_state().await.expect_err("nothing is listening");
    assert_eq!(
        error.code,
        ErrorCode::DaemonGone,
        "a socket with no daemon behind it is DaemonGone, not SourceUnreachable"
    );
}

#[tokio::test]
async fn artwork_agrees_across_transports() {
    let pair = spawn_pair().await;

    // The seeded rows have no cover on disk, so both sides have to agree on
    // the failure -- that is the whole request/stream/error path.
    let missing = api::ArtworkRequest {
        target: api::ArtworkTarget::Track("/lib/seed-0.flac".into()),
        hq: false,
    };
    let local = pair.local.artwork(missing.clone()).await;
    let wire = pair.wire.artwork(missing).await;
    assert_eq!(
        local.as_ref().err().map(|error| error.code),
        wire.as_ref().err().map(|error| error.code),
        "local {local:?} vs wire {wire:?}"
    );
    assert!(local.is_err(), "a track with no cover has no artwork");

    let unknown = api::ArtworkRequest {
        target: api::ArtworkTarget::Album("no-such-album".into()),
        hq: true,
    };
    assert_eq!(
        pair.local
            .artwork(unknown.clone())
            .await
            .err()
            .map(|e| e.code),
        pair.wire.artwork(unknown).await.err().map(|e| e.code),
    );
}

#[tokio::test]
async fn artists_are_keyed_by_identity_on_both_transports() {
    let pair = spawn_pair().await;
    let all = Page {
        offset: 0,
        limit: 100,
    };
    let credited = |key: &str, id: Option<&str>| Track {
        artist: "Ada".into(),
        artists: vec!["Ada".into()],
        credits: vec![match id {
            Some(id) => reader::ArtistCredit::linked("Ada", id),
            None => reader::ArtistCredit::unlinked("Ada"),
        }],
        ..track(key)
    };
    let tracks = [
        credited("/lib/ada-1a.flac", Some("ar-1")),
        credited("/lib/ada-1b.flac", Some("ar-1")),
        credited("/lib/ada-2.flac", Some("ar-2")),
        credited("/lib/ada-bare.flac", None),
    ];
    pair.database
        .upsert_tracks(&config::Source::default(), &tracks)
        .await
        .expect("seed credited tracks");

    let query = api::ArtistQuery::default();
    let artists = pair.local.artists(query.clone(), all).await.expect("local");
    assert_eq!(artists, pair.wire.artists(query, all).await.expect("wire"));
    let adas: Vec<&api::ArtistInfo> = artists
        .artists
        .iter()
        .filter(|artist| artist.name == "Ada")
        .collect();
    let mut counts: Vec<u32> = adas.iter().map(|artist| artist.track_count).collect();
    counts.sort();
    assert_eq!(
        counts,
        [1, 1, 2],
        "two ids and an unlinked name are three artists"
    );

    for ada in &adas {
        let local = pair.local.artist_tracks(ada.key.clone(), all).await;
        let wire = pair.wire.artist_tracks(ada.key.clone(), all).await;
        let local = local.expect("local");
        assert_eq!(local, wire.expect("wire"), "{}", ada.key);
        assert_eq!(
            local.total, ada.track_count,
            "a tile opens the tracks it counts"
        );
        let detail = pair.local.artist(ada.key.clone()).await.expect("local");
        assert_eq!(
            detail,
            pair.wire.artist(ada.key.clone()).await.expect("wire")
        );
        assert_eq!(
            detail.info, **ada,
            "the page names the artist the grid does"
        );
    }
    let linked = adas
        .iter()
        .find(|artist| artist.track_count == 2)
        .expect("ar-1");
    let one = pair
        .wire
        .artist_tracks(linked.key.clone(), all)
        .await
        .expect("wire");
    let mut keys: Vec<&str> = one.items.iter().map(|t| t.key.as_str()).collect();
    keys.sort();
    assert_eq!(keys, ["/lib/ada-1a.flac", "/lib/ada-1b.flac"]);
    assert!(
        one.items
            .iter()
            .all(|row| row.credits[0].key.as_ref() == Some(&linked.key)),
        "a row opens the artist the grid lists"
    );
}

#[tokio::test]
async fn an_artist_key_the_library_files_nothing_under_opens_nothing_on_both_transports() {
    let pair = spawn_pair().await;
    let all = Page {
        offset: 0,
        limit: 100,
    };
    for (key, code) in [
        ("", api::ErrorCode::NotFound),
        ("ar-1-of-another-server", api::ErrorCode::NotFound),
        ("0123456789abcdef", api::ErrorCode::NotFound),
    ] {
        let key = String::from(key);
        let local = pair.local.artist_tracks(key.clone(), all).await;
        let wire = pair.wire.artist_tracks(key.clone(), all).await;
        assert_eq!(local.err().map(|e| e.code), Some(code), "{key}");
        assert_eq!(wire.err().map(|e| e.code), Some(code), "{key}");
    }
}

#[tokio::test]
async fn library_reads_agree_across_transports() {
    let pair = spawn_pair().await;
    let all = Page {
        offset: 0,
        limit: 100,
    };

    let keys = vec![
        "/lib/seed-0.flac".to_string(),
        "/lib/seed-1.flac".to_string(),
    ];
    assert_eq!(
        pair.local
            .tracks_by_keys(keys.clone())
            .await
            .expect("local"),
        pair.wire.tracks_by_keys(keys).await.expect("wire"),
    );
    assert_eq!(
        pair.local
            .albums(api::AlbumQuery::default(), all)
            .await
            .expect("local"),
        pair.wire
            .albums(api::AlbumQuery::default(), all)
            .await
            .expect("wire"),
    );
    assert_eq!(
        pair.local
            .artists(api::ArtistQuery::default(), all)
            .await
            .expect("local"),
        pair.wire
            .artists(api::ArtistQuery::default(), all)
            .await
            .expect("wire"),
    );
    assert_eq!(
        pair.local.genres().await.expect("local"),
        pair.wire.genres().await.expect("wire"),
    );
    assert_eq!(
        pair.local.top_genre().await.expect("local"),
        pair.wire.top_genre().await.expect("wire"),
    );
    assert_eq!(
        pair.local.recent_tracks(all).await.expect("local"),
        pair.wire.recent_tracks(all).await.expect("wire"),
    );
    assert_eq!(
        pair.local.artist_sample_tracks(all).await.expect("local"),
        pair.wire.artist_sample_tracks(all).await.expect("wire"),
    );
    assert_eq!(
        pair.local
            .album_tracks("no-such-album".into(), all)
            .await
            .expect("local"),
        pair.wire
            .album_tracks("no-such-album".into(), all)
            .await
            .expect("wire"),
    );
    assert_eq!(
        pair.local
            .genre_tracks("rock".into(), all)
            .await
            .expect("local"),
        pair.wire
            .genre_tracks("rock".into(), all)
            .await
            .expect("wire"),
    );
    assert_eq!(
        pair.local
            .album("no-such-album".into())
            .await
            .expect("local"),
        pair.wire.album("no-such-album".into()).await.expect("wire"),
    );

    // The seeded rows carry no album id, so the paging metadata is what this
    // asserts: an empty page still reports the same totals on both sides.
    let local_tracks = pair
        .local
        .tracks_by_keys(vec![])
        .await
        .expect("local empty");
    assert!(local_tracks.is_empty());
}

/// Filters, orders and windows travel the wire intact: whatever the daemon answers in process, the socket answers the same.
#[tokio::test]
async fn listing_queries_filter_sort_and_page_the_same_on_both_transports() {
    use api::{AlbumQuery, ArtistQuery};
    use config::{AlbumSortField, ArtistSortField, SortCriterion, SortDirection};
    fn crit<F>(field: F, descending: bool) -> SortCriterion<F> {
        let direction = if descending {
            SortDirection::Desc
        } else {
            SortDirection::Asc
        };
        SortCriterion::new(field, direction)
    }

    let pair = spawn_pair().await;
    let source = config::Source::default();
    let on = |key: &str, title: &str, artist: &str, album_id: &str| Track {
        title: title.into(),
        artist: artist.into(),
        artists: vec![artist.into()],
        album_id: album_id.into(),
        ..track(key)
    };
    let album = |id: &str, title: &str, artist: &str, genre: &str, year: u16| reader::Album {
        id: id.into(),
        title: title.into(),
        artist: artist.into(),
        genre: genre.into(),
        year,
        cover_path: None,
        manual_cover: false,
        artist_id: None,
        artist_key: None,
    };
    pair.database
        .upsert_tracks(
            &source,
            &[
                on("/q/a1.flac", "Charlie", "Ada", "q-alpha"),
                on("/q/a2.flac", "Bravo", "Ada", "q-alpha"),
                on("/q/b1.flac", "Echo", "Boris", "q-beta"),
                on("/q/c1.flac", "Delta", "Cyd", "q-gamma"),
            ],
        )
        .await
        .expect("seed tracks");
    pair.database
        .upsert_albums(
            &source,
            &[
                album("q-alpha", "Alpha", "Ada", "Rock", 1991),
                album("q-beta", "beta", "Boris", "Jazz", 2005),
                album("q-gamma", "Gamma", "Cyd", "Rock", 2020),
            ],
        )
        .await
        .expect("seed albums");
    pair.database
        .set_offline_track("/q/b1.flac", Some("/offline/b1"))
        .await
        .expect("mark a download");

    let window = Page {
        offset: 1,
        limit: 2,
    };
    let everything = Page {
        offset: 0,
        limit: 100,
    };
    let sorted = |field, descending| vec![crit(field, descending)];
    let queries = [
        AlbumQuery::default(),
        AlbumQuery {
            search: Some("ALP".into()),
            ..Default::default()
        },
        AlbumQuery {
            genre: Some("Rock".into()),
            sort: sorted(AlbumSortField::Year, true),
            ..Default::default()
        },
        AlbumQuery {
            year_from: Some(2000),
            year_to: Some(2010),
            ..Default::default()
        },
        AlbumQuery {
            sort: vec![
                crit(AlbumSortField::TrackCount, true),
                crit(AlbumSortField::Title, false),
            ],
            ..Default::default()
        },
        AlbumQuery {
            sort: sorted(AlbumSortField::RecentlyAdded, true),
            ..Default::default()
        },
    ];
    for query in queries {
        for page in [everything, window] {
            let local = pair.local.albums(query.clone(), page).await.expect("local");
            let wire = pair.wire.albums(query.clone(), page).await.expect("wire");
            assert_eq!(local, wire, "{query:?} {page:?}");
        }
    }

    let by_year = AlbumQuery {
        sort: sorted(AlbumSortField::Year, true),
        ..Default::default()
    };
    let listed = pair.wire.albums(by_year, window).await.expect("wire");
    assert_eq!(
        listed.total, 3,
        "the total is the filtered set, not the window"
    );
    let titles: Vec<&str> = listed.albums.iter().map(|a| a.title.as_str()).collect();
    assert_eq!(titles, ["beta", "Alpha"], "year descending, then cut");
    let rock = AlbumQuery {
        genre: Some("Rock".into()),
        ..Default::default()
    };
    assert_eq!(
        pair.wire
            .albums(rock, everything)
            .await
            .expect("wire")
            .total,
        2
    );

    let ada = pair
        .wire
        .artists(
            ArtistQuery {
                search: Some("ada".into()),
                ..Default::default()
            },
            everything,
        )
        .await
        .expect("wire")
        .artists
        .remove(0);
    assert_eq!((ada.track_count, ada.album_count), (2, 1));
    let billed = AlbumQuery {
        artist_key: Some(ada.key),
        ..Default::default()
    };
    let local = pair
        .local
        .albums(billed.clone(), everything)
        .await
        .expect("local");
    assert_eq!(
        local,
        pair.wire.albums(billed, everything).await.expect("wire")
    );
    assert_eq!(local.albums.len(), 1);

    let artist_queries = [
        ArtistQuery::default(),
        ArtistQuery {
            search: Some("o".into()),
            ..Default::default()
        },
        ArtistQuery {
            sort: vec![crit(ArtistSortField::Tracks, true)],
            ..Default::default()
        },
        ArtistQuery {
            sort: vec![crit(ArtistSortField::Albums, false)],
            ..Default::default()
        },
        ArtistQuery {
            sort: vec![crit(ArtistSortField::Name, true)],
            ..Default::default()
        },
    ];
    for query in artist_queries {
        for page in [everything, window] {
            let local = pair
                .local
                .artists(query.clone(), page)
                .await
                .expect("local");
            let wire = pair.wire.artists(query.clone(), page).await.expect("wire");
            assert_eq!(local, wire, "{query:?} {page:?}");
        }
    }
    let by_tracks = ArtistQuery {
        sort: vec![crit(ArtistSortField::Tracks, true)],
        ..Default::default()
    };
    let top = pair
        .wire
        .artists(by_tracks, everything)
        .await
        .expect("wire");
    assert_eq!(top.artists[0].name, "Ada");

    let filters = [
        TrackFilter {
            downloaded: Some(true),
            ..Default::default()
        },
        TrackFilter {
            downloaded: Some(false),
            ..Default::default()
        },
        TrackFilter {
            year_from: Some(1990),
            year_to: Some(1999),
            ..Default::default()
        },
        TrackFilter {
            sort: api::TrackSort::Title,
            reverse: true,
            ..Default::default()
        },
        TrackFilter {
            genre: Some("Rock".into()),
            year_from: Some(2000),
            reverse: true,
            ..Default::default()
        },
    ];
    for filter in filters {
        for page in [everything, window] {
            let local = pair
                .local
                .tracks(filter.clone(), page)
                .await
                .expect("local");
            let wire = pair.wire.tracks(filter.clone(), page).await.expect("wire");
            assert_eq!(local, wire, "{filter:?} {page:?}");
        }
    }
    let downloaded = pair
        .wire
        .tracks(
            TrackFilter {
                downloaded: Some(true),
                ..Default::default()
            },
            everything,
        )
        .await
        .expect("wire");
    let keys: Vec<&str> = downloaded.items.iter().map(|t| t.key.as_str()).collect();
    assert_eq!(keys, ["/q/b1.flac"]);
    let nineties = pair
        .wire
        .tracks(
            TrackFilter {
                year_to: Some(1999),
                ..Default::default()
            },
            everything,
        )
        .await
        .expect("wire");
    assert_eq!(nineties.total, 2, "the two tracks of the 1991 album");
    let rock_since_2000 = pair
        .wire
        .tracks(
            TrackFilter {
                genre: Some("Rock".into()),
                year_from: Some(2000),
                ..Default::default()
            },
            everything,
        )
        .await
        .expect("wire");
    let keys: Vec<&str> = rock_since_2000
        .items
        .iter()
        .map(|t| t.key.as_str())
        .collect();
    assert_eq!(
        keys,
        ["/q/c1.flac"],
        "a genre listing honours the year bounds too"
    );
    let reversed = pair
        .wire
        .tracks(
            TrackFilter {
                sort: api::TrackSort::Title,
                reverse: true,
                ..Default::default()
            },
            Page {
                offset: 0,
                limit: 1,
            },
        )
        .await
        .expect("wire");
    assert_eq!(
        reversed.items[0].title, "Echo",
        "a reversed title order starts at the last title"
    );
}

#[tokio::test]
async fn playlists_round_trip_across_transports() {
    let pair = spawn_pair().await;

    let id = pair
        .wire
        .create_playlist("Over the wire".into(), vec!["/lib/seed-0.flac".into()])
        .await
        .expect("create over the wire");

    // Both sides see the same catalog, and the wire's write is visible locally.
    let local = pair.local.playlists().await.expect("local catalog");
    let wire = pair.wire.playlists().await.expect("wire catalog");
    assert_eq!(local, wire);
    let created = local
        .playlists
        .iter()
        .find(|playlist| playlist.id == id)
        .expect("the created playlist is in the catalog");
    assert_eq!(created.name, "Over the wire");
    assert_eq!(created.track_keys, vec!["/lib/seed-0.flac".to_string()]);

    pair.wire
        .add_playlist_tracks(id.clone(), vec!["/lib/seed-1.flac".into()])
        .await
        .expect("add over the wire");
    pair.local
        .rename_playlist(id.clone(), "Renamed locally".into())
        .await
        .expect("rename locally");

    let catalog = pair.wire.playlists().await.expect("catalog");
    let playlist = catalog
        .playlists
        .iter()
        .find(|playlist| playlist.id == id)
        .expect("still there");
    assert_eq!(playlist.name, "Renamed locally");
    assert_eq!(playlist.track_keys.len(), 2);

    pair.wire
        .remove_playlist_track(id.clone(), 0)
        .await
        .expect("remove over the wire");
    let catalog = pair.local.playlists().await.expect("catalog");
    let playlist = catalog
        .playlists
        .iter()
        .find(|playlist| playlist.id == id)
        .expect("still there");
    assert_eq!(
        playlist.track_keys,
        vec!["/lib/seed-1.flac".to_string()],
        "position-addressed removal took the first entry"
    );

    // Folders are local organisation; a playlist moves in and back out.
    let folder = pair
        .wire
        .create_playlist_folder("Shelf".into())
        .await
        .expect("create folder");
    pair.wire
        .move_playlist(id.clone(), Some(folder.clone()))
        .await
        .expect("move in");
    let catalog = pair.local.playlists().await.expect("catalog");
    assert!(
        catalog
            .folders
            .iter()
            .any(|entry| entry.id == folder && entry.playlist_ids.contains(&id)),
        "the folder holds the playlist: {:?}",
        catalog.folders
    );

    pair.local
        .delete_playlist(id.clone())
        .await
        .expect("delete locally");
    let catalog = pair.wire.playlists().await.expect("catalog");
    assert!(!catalog.playlists.iter().any(|playlist| playlist.id == id));
}

/// A row's artwork ref is the client's whole decision: absent means there is
/// no picture, so a grid draws its placeholder without a request. Both
/// transports must agree on that, and on the version that keys the cache.
#[tokio::test]
async fn artwork_refs_agree_across_transports() {
    let pair = spawn_pair().await;

    // The seeded tracks have no cover, so no row may claim one.
    let local = pair
        .local
        .tracks(TrackFilter::default(), Page::default())
        .await
        .expect("local tracks");
    let wire = pair
        .wire
        .tracks(TrackFilter::default(), Page::default())
        .await
        .expect("wire tracks");
    assert_eq!(local.items, wire.items, "rows agree across transports");
    assert!(
        local.items.iter().all(|track| track.artwork.is_none()),
        "a track with no cover advertises none: {:?}",
        local.items
    );

    // Asking anyway is the same not-found on both sides, which is what makes
    // "absent means do not ask" safe rather than merely conventional.
    let request = api::ArtworkRequest {
        target: api::ArtworkTarget::Track("/lib/seed-0.flac".into()),
        hq: false,
    };
    assert_eq!(
        pair.local
            .artwork(request.clone())
            .await
            .err()
            .map(|error| error.code),
        pair.wire
            .artwork(request)
            .await
            .err()
            .map(|error| error.code),
    );

    // A cover appears: the row starts advertising one, and its version is
    // stable across reads and transports.
    // Undecodable bytes on purpose: the service falls back to serving the
    // file as-is, which is the path an unusual cover format takes anyway.
    let cover = pair._dir.path().join("cover.png");
    std::fs::write(&cover, b"\x89PNG\r\n\x1a\nnot-really-an-image").expect("write cover");
    let mut with_art = track("/lib/seed-0.flac");
    with_art.cover = Some(cover.to_string_lossy().into_owned());
    pair.database
        .upsert_tracks(&config::Source::default(), &[with_art])
        .await
        .expect("re-seed with a cover");

    let local = pair
        .local
        .tracks(TrackFilter::default(), Page::default())
        .await
        .expect("local tracks");
    let wire = pair
        .wire
        .tracks(TrackFilter::default(), Page::default())
        .await
        .expect("wire tracks");
    let art = local
        .items
        .iter()
        .find(|track| track.key == "/lib/seed-0.flac")
        .and_then(|track| track.artwork.clone())
        .expect("the row now advertises artwork");
    assert_eq!(
        art.target,
        api::ArtworkTarget::Track("/lib/seed-0.flac".into())
    );
    assert_eq!(local.items, wire.items, "the version crosses the wire");

    // And it now serves bytes, on both transports.
    let request = api::ArtworkRequest {
        target: art.target.clone(),
        hq: false,
    };
    let bytes = pair
        .wire
        .artwork(request.clone())
        .await
        .expect("artwork over the wire");
    assert!(!bytes.bytes.is_empty());
    assert_eq!(
        bytes.content_type,
        pair.local
            .artwork(request)
            .await
            .expect("artwork locally")
            .content_type
    );
}

/// The queue snapshot is what a frontend mirrors: the rows in play order plus
/// the permutation behind them. `Insert` and `JumpPhysical` are the two edits
/// a queue view makes that a paged window cannot express.
#[tokio::test]
async fn queue_snapshot_and_edits_agree_across_transports() {
    let pair = spawn_pair().await;

    pair.local
        .set_queue(SetQueueRequest {
            mode: QueueMode::Replace,
            context: QueueContext::Tracks {
                keys: vec!["/lib/a.flac".into(), "/lib/b.flac".into()],
            },
            start_index: Some(0),
            shuffle: Some(false),
        })
        .await
        .expect("seed the queue");

    let local = pair.local.queue_snapshot().await.expect("local snapshot");
    let wire = pair.wire.queue_snapshot().await.expect("wire snapshot");
    assert_eq!(local.items.len(), 2);
    assert_eq!(local.position, Some(0));
    assert_eq!(
        local.items.iter().map(|item| &item.key).collect::<Vec<_>>(),
        wire.items.iter().map(|item| &item.key).collect::<Vec<_>>(),
    );

    // Insert lands where it was asked to, without disturbing what plays.
    pair.wire
        .queue_edit(QueueEdit::Insert {
            index: 1,
            keys: vec!["/lib/c.flac".into()],
        })
        .await
        .expect("insert over the wire");
    let snapshot = pair.local.queue_snapshot().await.expect("snapshot");
    assert_eq!(
        snapshot
            .items
            .iter()
            .map(|item| item.key.as_str())
            .collect::<Vec<_>>(),
        vec!["/lib/a.flac", "/lib/c.flac", "/lib/b.flac"],
    );
    assert_eq!(snapshot.position, Some(0), "the playing track did not move");

    // A physical jump names a position in the unshuffled queue.
    pair.wire
        .queue_edit(QueueEdit::JumpPhysical { index: 2 })
        .await
        .expect("jump over the wire");
    let snapshot = pair.wire.queue_snapshot().await.expect("snapshot");
    assert_eq!(snapshot.position, Some(2));

    // Out of range is an error, not a silent no-op, on both transports.
    let edit = QueueEdit::JumpPhysical { index: 99 };
    assert_eq!(
        pair.local
            .queue_edit(edit.clone())
            .await
            .err()
            .map(|error| error.code),
        pair.wire
            .queue_edit(edit)
            .await
            .err()
            .map(|error| error.code),
    );
}

/// Neither transport may queue nothing and answer success.
#[tokio::test]
async fn a_queue_from_an_unresolved_id_is_refused_identically() {
    let pair = spawn_pair().await;

    for context in [
        QueueContext::Album {
            id: "MPREunknown".into(),
        },
        QueueContext::Playlist {
            id: "PLunsaved".into(),
        },
    ] {
        let request = SetQueueRequest {
            mode: QueueMode::Replace,
            context: context.clone(),
            start_index: Some(0),
            shuffle: Some(false),
        };
        let local = pair.local.set_queue(request.clone()).await.err();
        let wire = pair.wire.set_queue(request).await.err();
        assert!(local.is_some(), "{context:?}");
        assert_eq!(local.map(|e| e.code), wire.map(|e| e.code), "{context:?}");
    }
    let snapshot = pair.local.queue_snapshot().await.expect("snapshot");
    assert!(snapshot.items.is_empty(), "nothing was queued");
}

/// A local library has no account to picture.
#[tokio::test]
async fn an_account_picture_is_absent_identically_without_an_account() {
    let pair = spawn_pair().await;

    let local = pair.local.sources().await.expect("local sources");
    let wire = pair.wire.sources().await.expect("wire sources");
    assert!(local.iter().all(|source| source.avatar.is_none()));
    assert_eq!(
        local.iter().map(|s| &s.avatar).collect::<Vec<_>>(),
        wire.iter().map(|s| &s.avatar).collect::<Vec<_>>()
    );

    let active = local.iter().find(|s| s.active).expect("an active source");
    let request = api::ArtworkRequest {
        target: api::ArtworkTarget::Account(active.id.clone()),
        hq: false,
    };
    let local = pair.local.artwork(request.clone()).await.err();
    let wire = pair.wire.artwork(request).await.err();
    assert_eq!(local.as_ref().map(|e| e.code), Some(ErrorCode::NotFound));
    assert_eq!(local.map(|e| e.code), wire.map(|e| e.code));
}

/// With shuffle on, both transports list the rows in play order and `shuffle_order` maps them back.
#[tokio::test]
async fn shuffled_snapshot_is_play_order_on_both_transports() {
    let pair = spawn_pair().await;
    let keys: Vec<String> = (0..8).map(|i| format!("/lib/s{i}.flac")).collect();
    pair.local
        .set_queue(SetQueueRequest {
            mode: QueueMode::Replace,
            context: QueueContext::Tracks { keys: keys.clone() },
            start_index: Some(0),
            shuffle: Some(true),
        })
        .await
        .expect("seed a shuffled queue");

    let local = pair.local.queue_snapshot().await.expect("local snapshot");
    let wire = pair.wire.queue_snapshot().await.expect("wire snapshot");
    assert!(local.shuffle);
    assert_eq!(local, wire);
    assert_eq!(local.shuffle_order.len(), keys.len());
    for (position, physical) in local.shuffle_order.iter().enumerate() {
        assert_eq!(local.items[position].key, keys[*physical as usize]);
    }
    let window = pair
        .wire
        .queue_window(Page::default())
        .await
        .expect("window");
    assert_eq!(
        window
            .items
            .iter()
            .map(|i| &i.track.key)
            .collect::<Vec<_>>(),
        wire.items.iter().map(|t| &t.key).collect::<Vec<_>>(),
        "the window and the snapshot agree on play order"
    );
}

/// Neither service is configured in this harness, so both transports must
/// agree on saying so rather than one erroring and the other answering
/// empty -- the failure mode a second implementation would inherit.
#[tokio::test]
async fn catalog_and_radio_report_absence_identically() {
    let pair = spawn_pair().await;

    assert_eq!(
        pair.local.catalog(None).await.err().map(|e| e.code),
        pair.wire.catalog(None).await.err().map(|e| e.code),
    );

    let request = api::CatalogDetailRequest::new(api::CatalogItemKind::Album, "MPRE1");
    assert_eq!(
        pair.local
            .catalog_detail(request.clone())
            .await
            .err()
            .map(|e| e.code),
        pair.wire
            .catalog_detail(request)
            .await
            .err()
            .map(|e| e.code),
    );

    assert_eq!(
        pair.local.radio_stations().await.err().map(|e| e.code),
        pair.wire.radio_stations().await.err().map(|e| e.code),
    );
    assert_eq!(
        pair.local
            .pin_radio_station("st-1".into(), true)
            .await
            .err()
            .map(|e| e.code),
        pair.wire
            .pin_radio_station("st-1".into(), true)
            .await
            .err()
            .map(|e| e.code),
    );

    // A URL that is not a registry is refused as bad input, not reported as a
    // registry with no stations, and says so the same way on both transports.
    let url = "file:///nonexistent/registry.json".to_string();
    assert_eq!(
        pair.local
            .validate_radio_registry(url.clone())
            .await
            .err()
            .map(|e| e.code),
        pair.wire
            .validate_radio_registry(url)
            .await
            .err()
            .map(|e| e.code),
    );
}

/// A folder library starts no radio, so "For you" is unavailable rather than an error, on both transports.
#[tokio::test]
async fn for_you_without_track_radio_hides_the_row_on_both_transports() {
    let pair = spawn_pair().await;
    pair.local
        .set_favorite("/lib/seed-0.flac".into(), true)
        .await
        .expect("a like to seed from");
    let hidden = api::ForYou::Unavailable(api::ForYouUnavailable::NoTrackRadio);

    assert_eq!(pair.local.for_you().await, Ok(hidden.clone()));
    assert_eq!(pair.wire.for_you().await, Ok(hidden.clone()));
    assert_eq!(pair.local.refresh_for_you().await, Ok(hidden.clone()));
    assert_eq!(pair.wire.refresh_for_you().await, Ok(hidden));
}

/// A source across the internet going quiet while the checks fail too is the internet down, on both transports, and a switch back to it is refused.
#[tokio::test]
async fn the_internet_going_down_reaches_both_transports_and_refuses_a_switch() {
    use futures_util::StreamExt;
    let pair = spawn_pair().await;
    for api in [&pair.local as &dyn KopuzApi, &pair.wire] {
        assert_eq!(
            api.network_status().await,
            Ok(api::NetworkStatus::default())
        );
    }
    let closed = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        format!("http://{}/", listener.local_addr().expect("address"))
    };
    let report = pair
        .wire
        .edit_config(vec![config::SettingsEdit::System(
            config::SystemEdit::ConnectivityCheckUrls(vec![closed]),
        )])
        .await
        .expect("edit");
    assert_eq!(report.outcomes, [api::EditOutcome::Applied]);
    let far = pair
        .wire
        .upsert_source(api::SourceDraft {
            name: "Far away".into(),
            service: "jellyfin".into(),
            values: vec![api::FieldValue::new("url", "https://jellyfin.invalid")],
            ..Default::default()
        })
        .await
        .expect("add");
    pair.wire
        .provision_credentials(api::CredentialProvision {
            server_id: far.id.clone(),
            secret: "token".into(),
            user_id: Some("ada".into()),
            browser: None,
        })
        .await
        .expect("sign in");
    let mut events = pair.wire.events();

    assert_eq!(
        pair.wire.validate_source(far.id.clone()).await,
        Ok(api::SourceState::Offline)
    );

    let down = tokio::time::timeout(Duration::from_secs(20), async {
        while let Some(event) = events.next().await {
            if let ApiEvent::NetworkStatus(status) = event {
                return status;
            }
        }
        panic!("the event stream ended");
    })
    .await
    .expect("the check ran");
    assert!(down.internet_down);
    assert_eq!(
        down.left_source, None,
        "the active folder library needs no move"
    );
    assert_eq!(pair.local.network_status().await, Ok(down));
    let refused = pair
        .wire
        .switch_source(far.id)
        .await
        .expect_err("the internet is down");
    assert_eq!(refused.reason, Some(api::ErrorReason::NetworkDown));
}

/// A folder library's Discover rows agree on both transports and hold until a refresh, even past a play.
#[tokio::test]
async fn discover_rows_agree_across_transports_and_hold_until_refreshed() {
    let pair = spawn_pair().await;
    let local = config::Source::default();
    let names: Vec<String> = (0..30).map(|n| format!("al-{n}")).collect();
    let tracks: Vec<Track> = names
        .iter()
        .map(|name| Track {
            album_id: name.clone(),
            ..track(&format!("/lib/{name}.flac"))
        })
        .collect();
    pair.database.upsert_tracks(&local, &tracks).await.unwrap();
    let albums: Vec<reader::Album> = names
        .iter()
        .map(|name| reader::Album {
            id: name.clone(),
            title: name.clone(),
            artist: "Ada".into(),
            genre: String::new(),
            year: 0,
            cover_path: None,
            manual_cover: false,
            artist_id: None,
            artist_key: None,
        })
        .collect();
    pair.database.upsert_albums(&local, &albums).await.unwrap();
    let active = pair
        .wire
        .sources()
        .await
        .unwrap()
        .into_iter()
        .find(|source| source.active)
        .expect("an active source");
    assert!(active.capabilities.complete_play_history);

    let first = pair.local.discover_rows().await.unwrap();
    assert_eq!(pair.wire.discover_rows().await, Ok(first.clone()));
    assert!(first.mixes.is_empty());
    assert!(!first.never_played.is_empty());
    let picked = first.never_played[0].id.clone();
    pair.database
        .bump_listen_count(&local, &format!("/lib/{picked}.flac"))
        .await
        .unwrap();

    assert_eq!(
        pair.wire.discover_rows().await,
        Ok(first),
        "held until refreshed"
    );
    let refreshed = pair.wire.refresh_discover_rows().await.unwrap();
    assert!(
        refreshed
            .never_played
            .iter()
            .all(|album| album.id != picked),
        "a played album leaves the row once it is picked again"
    );
    assert_eq!(pair.local.discover_rows().await, Ok(refreshed));
}

/// An unsaved draft is probed alike on both transports: folders counted, a server asked once, a streaming service left alone.
#[tokio::test]
async fn a_draft_probe_agrees_across_transports() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let pair = spawn_pair().await;
    let music = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(music.path().join("album")).expect("mkdir");
    std::fs::write(music.path().join("album/one.flac"), b"").expect("write");
    std::fs::write(music.path().join("two.mp3"), b"").expect("write");
    let missing = music.path().join("missing").to_string_lossy().into_owned();
    let folders = api::SourceDraft {
        name: "Mine".into(),
        service: "folders".into(),
        values: vec![api::FieldValue::new(
            "directories",
            api::encode_directories(&[
                music.path().to_string_lossy().into_owned(),
                missing.clone(),
            ]),
        )],
        ..Default::default()
    };
    let expected = api::DraftProbe::Folders(vec![
        api::FolderProbe {
            path: music.path().to_string_lossy().into_owned(),
            exists: true,
            readable: true,
            audio_files: 2,
            at_least: false,
        },
        api::FolderProbe {
            path: missing,
            ..Default::default()
        },
    ]);
    assert_eq!(
        pair.local.probe_source_draft(folders.clone()).await,
        Ok(expected.clone())
    );
    assert_eq!(pair.wire.probe_source_draft(folders).await, Ok(expected));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut buffer = [0u8; 1024];
            let _ = socket.read(&mut buffer).await;
            let _ = socket
                .write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await;
        }
    });
    let server = api::SourceDraft {
        name: "Jelly".into(),
        service: "jellyfin".into(),
        values: vec![api::FieldValue::new("url", &address)],
        ..Default::default()
    };
    let answered = api::DraftProbe::Server(api::ServerProbe::Status(404));
    assert_eq!(
        pair.local.probe_source_draft(server.clone()).await,
        Ok(answered.clone())
    );
    assert_eq!(pair.wire.probe_source_draft(server).await, Ok(answered));

    let streaming = api::SourceDraft {
        name: "YT".into(),
        service: "ytmusic".into(),
        ..Default::default()
    };
    assert_eq!(
        pair.wire.probe_source_draft(streaming).await,
        Ok(api::DraftProbe::NotProbeable)
    );
    let bad = api::SourceDraft {
        name: "Jelly".into(),
        service: "jellyfin".into(),
        values: vec![api::FieldValue::new("url", "jelly.local")],
        ..Default::default()
    };
    let refused = pair
        .wire
        .probe_source_draft(bad)
        .await
        .expect_err("not an address");
    assert_eq!(refused.reason, Some(api::ErrorReason::InvalidServerUrl));
}

/// Opening a page the source does not have fails the same way on both transports.
#[tokio::test]
async fn catalog_pages_agree_across_transports() {
    let pair = spawn_pair().await;

    let active = |sources: Vec<api::SourceInfo>| {
        sources
            .into_iter()
            .find(|source| source.active)
            .expect("an active source")
            .capabilities
    };
    let local = active(pair.local.sources().await.expect("local sources"));
    let wire = active(pair.wire.sources().await.expect("wire sources"));
    assert_eq!(local, wire);
    assert!(local.pages.is_empty(), "a local library declares no pages");

    let request = api::CatalogDetailRequest::page("FEmusic_home");
    let local = pair.local.catalog_detail(request.clone()).await;
    let wire = pair.wire.catalog_detail(request).await;
    assert_eq!(
        local.as_ref().err().map(|e| e.code),
        Some(ErrorCode::Unsupported)
    );
    assert_eq!(local.err().map(|e| e.code), wire.err().map(|e| e.code));
}

/// An unknown filter fails the same way on both transports, not empty on one.
#[tokio::test]
async fn search_filters_agree_across_transports() {
    let pair = spawn_pair().await;

    let plain = api::SearchRequest::new("seed");
    let local = pair
        .local
        .search(plain.clone())
        .await
        .expect("local search");
    let wire = pair.wire.search(plain).await.expect("wire search");
    assert_eq!(local, wire);
    assert!(
        !local.tracks.is_empty(),
        "the plain search still finds rows"
    );
    assert!(local.shelves.is_empty() && local.continuation.is_none());

    let caps = pair
        .wire
        .sources()
        .await
        .expect("sources")
        .into_iter()
        .find(|source| source.active)
        .expect("an active source")
        .capabilities;
    assert!(caps.search_filters.is_empty());

    let filtered = api::SearchRequest::filtered("seed", "songs");
    assert_eq!(
        pair.local
            .search(filtered.clone())
            .await
            .err()
            .map(|e| e.code),
        pair.wire.search(filtered).await.err().map(|e| e.code),
    );
    let local = pair.local.search_suggestions("se".into()).await;
    let wire = pair.wire.search_suggestions("se".into()).await;
    assert_eq!(local.map_err(|e| e.code), wire.map_err(|e| e.code));
}

/// Deleting from disk is the one API call that destroys something outside
/// the database, so its guard has to hold identically on both transports.
#[tokio::test]
async fn mutations_agree_across_transports() {
    let pair = spawn_pair().await;

    // The seeded tracks name paths that do not exist and are outside any
    // configured root, so a from-disk delete is refused rather than
    // half-applied.
    let keys = vec!["/lib/seed-0.flac".to_string()];
    let local = pair.local.delete_tracks(keys.clone(), true).await;
    let wire = pair.wire.delete_tracks(keys.clone(), true).await;
    assert!(local.is_err(), "outside the library roots: {local:?}");
    assert_eq!(
        local.err().map(|error| error.code),
        wire.err().map(|error| error.code),
    );
    assert_eq!(
        pair.local
            .tracks(TrackFilter::default(), Page::default())
            .await
            .expect("tracks")
            .total,
        2,
        "a refused delete changed nothing"
    );

    // Tags are only editable on a local file that exists; the failure is the
    // same either way.
    let patch = api::TrackMetadataPatch {
        key: "/lib/seed-0.flac".into(),
        title: Some("renamed".into()),
        ..Default::default()
    };
    assert_eq!(
        pair.local
            .update_track_metadata(patch.clone())
            .await
            .err()
            .map(|error| error.code),
        pair.wire
            .update_track_metadata(patch)
            .await
            .err()
            .map(|error| error.code),
    );

    // Artwork has to decode before it is stored, on either transport.
    let upload = api::ArtworkUpload {
        target: api::ArtworkTarget::Album("album-1".into()),
        content_type: "image/png".into(),
        bytes: b"\x89PNG\r\n\x1a\nnot an image".to_vec(),
    };
    let local = pair.local.upload_artwork(upload.clone()).await;
    assert_eq!(
        local.as_ref().err().map(|error| error.code),
        Some(ErrorCode::InvalidInput),
        "got {local:?}"
    );
    assert_eq!(
        local.err().map(|error| error.code),
        pair.wire
            .upload_artwork(upload)
            .await
            .err()
            .map(|error| error.code),
    );
}

/// A source row is what a settings page renders. Both transports must agree
/// on it, and neither may carry a credential back out.
#[tokio::test]
async fn sources_agree_across_transports_and_carry_no_secret() {
    let pair = spawn_pair().await;
    wait_probed(&pair).await;

    let local = pair.local.sources().await.expect("local sources");
    let wire = pair.wire.sources().await.expect("wire sources");
    assert_eq!(local, wire);
    assert_eq!(local.len(), 1, "just the default local library: {local:?}");
    assert!(local[0].active, "the default library is active");
    assert!(local[0].authenticated, "a local library needs no sign-in");
    assert_eq!(local[0].service.id, "folders");
    assert!(!local[0].needs_network);
    assert_eq!(local[0].sign_in, api::SignInKind::None);
    assert!(
        local[0]
            .settings
            .iter()
            .any(|field| matches!(field.kind, api::FieldKind::Directories)),
        "its folders are a settings row: {:?}",
        local[0].settings
    );

    // Adding a server is visible to both, and provisioning a credential
    // reports authentication without echoing the secret.
    let draft = api::SourceDraft {
        name: "Home".into(),
        service: "jellyfin".into(),
        values: vec![api::FieldValue::new("url", "https://jelly.example")],
        ..Default::default()
    };
    let added = pair.wire.upsert_source(draft).await.expect("add server");
    assert!(!added.authenticated, "a new server has no credentials yet");
    assert_eq!(added.detail.as_deref(), Some("https://jelly.example"));
    assert_eq!(
        added.sign_in,
        api::SignInKind::Password,
        "a Jellyfin server signs in with a username and a password"
    );

    pair.local
        .provision_credentials(api::CredentialProvision {
            server_id: added.id.clone(),
            secret: "a-token-nobody-should-see".into(),
            user_id: Some("alice".into()),
            browser: None,
        })
        .await
        .expect("provision");

    let sources = pair.wire.sources().await.expect("wire sources");
    let server = sources
        .iter()
        .find(|source| source.id == added.id)
        .expect("the server is listed");
    assert!(server.authenticated, "it is signed in now");
    assert_eq!(server.sign_in, api::SignInKind::None, "nothing left to do");
    let rendered = format!("{sources:?}");
    assert!(
        !rendered.contains("a-token-nobody-should-see"),
        "no response may carry the secret: {rendered}"
    );

    // A bad draft is refused identically.
    let bad = api::SourceDraft {
        name: "No URL".into(),
        service: "jellyfin".into(),
        values: vec![api::FieldValue::new("url", "not-a-url")],
        ..Default::default()
    };
    assert_eq!(
        pair.local
            .upsert_source(bad.clone())
            .await
            .err()
            .map(|error| error.code),
        pair.wire.upsert_source(bad).await.err().map(|e| e.code),
    );

    pair.wire
        .delete_source(added.id.clone())
        .await
        .expect("delete");
    assert_eq!(pair.local.sources().await.expect("sources").len(), 1);
}

/// A folder source is created, edited and deleted through the same calls as a
/// server, with its folders carried as one field value.
#[tokio::test]
async fn a_folder_source_is_managed_through_the_generic_source_calls() {
    let pair = spawn_pair().await;
    let music = tempfile::tempdir().expect("tempdir");
    let more = tempfile::tempdir().expect("tempdir");
    let path = |dir: &tempfile::TempDir| dir.path().display().to_string();
    let folders =
        |paths: &[String]| api::FieldValue::new("directories", api::encode_directories(paths));

    let services = pair.wire.services().await.expect("services");
    let offered = services
        .iter()
        .find(|service| service.id == "folders")
        .expect("folders are offered like any service");
    assert!(
        offered
            .fields
            .iter()
            .any(|field| field.key == "directories"
                && matches!(field.kind, api::FieldKind::Directories)),
        "{offered:?}"
    );

    let empty = api::SourceDraft {
        name: "Jazz".into(),
        service: "folders".into(),
        values: vec![folders(&[])],
        ..Default::default()
    };
    let check = pair
        .wire
        .check_source_draft(empty.clone())
        .await
        .expect("check");
    assert_eq!(
        check,
        pair.local
            .check_source_draft(empty.clone())
            .await
            .expect("check")
    );
    assert!(
        check
            .problems
            .iter()
            .any(|problem| problem.field.as_deref() == Some("directories")),
        "a folder source needs a folder: {check:?}"
    );
    assert_eq!(
        pair.wire.upsert_source(empty).await.err().map(|e| e.code),
        Some(ErrorCode::InvalidInput)
    );

    let created = pair
        .wire
        .upsert_source(api::SourceDraft {
            name: "Jazz".into(),
            service: "folders".into(),
            values: vec![folders(&[path(&music)])],
            ..Default::default()
        })
        .await
        .expect("create");
    assert_eq!(created.name, "Jazz");
    assert_eq!(created.service.id, "folders");
    assert!(created.authenticated && !created.needs_network);
    assert_eq!(
        api::spec_value(&created.settings, "directories").map(api::decode_directories),
        Some(vec![path(&music)])
    );
    wait_probed(&pair).await;
    let listed = pair.local.sources().await.expect("sources");
    assert_eq!(listed, pair.wire.sources().await.expect("sources"));
    assert_eq!(listed.len(), 2, "{listed:?}");

    let renamed = pair
        .local
        .upsert_source(api::SourceDraft {
            id: Some(created.id.clone()),
            name: "Bebop".into(),
            service: "folders".into(),
            values: vec![folders(&[path(&music), path(&more)])],
            ..Default::default()
        })
        .await
        .expect("edit");
    assert_eq!(renamed.id, created.id);
    assert_eq!(renamed.name, "Bebop");
    assert_eq!(
        api::spec_value(&renamed.settings, "directories").map(api::decode_directories),
        Some(vec![path(&music), path(&more)])
    );

    let narrowed = pair
        .wire
        .set_source_settings(created.id.clone(), vec![folders(&[path(&more)])])
        .await
        .expect("set folders");
    assert_eq!(
        api::spec_value(&narrowed.settings, "directories").map(api::decode_directories),
        Some(vec![path(&more)])
    );
    assert_eq!(narrowed.name, "Bebop", "the name is left alone");

    assert_eq!(
        pair.wire
            .validate_source(created.id.clone())
            .await
            .expect("validate"),
        api::SourceState::Online
    );

    let filed = config::Source::from_column(&created.id);
    pair.database
        .upsert_tracks(&filed, &[track("/jazz/a.flac"), track("/jazz/b.flac")])
        .await
        .expect("seed the folder source");
    let library_before = pair
        .database
        .tracks_count(&db::TrackFilter::new(config::Source::default()))
        .await
        .expect("count");

    pair.wire
        .delete_source(created.id.clone())
        .await
        .expect("delete");
    let left = pair.local.sources().await.expect("sources");
    assert_eq!(left.len(), 1, "{left:?}");
    assert_eq!(
        pair.database
            .tracks_count(&db::TrackFilter::new(filed.clone()))
            .await
            .expect("count"),
        0,
        "a deleted folder source leaves no tracks behind"
    );
    assert!(
        pair.database
            .artists(&filed)
            .await
            .expect("artists")
            .is_empty(),
        "nor artists"
    );
    assert_eq!(
        pair.database
            .tracks_count(&db::TrackFilter::new(config::Source::default()))
            .await
            .expect("count"),
        library_before,
        "the other library is untouched"
    );
    assert_eq!(
        pair.local
            .delete_source(created.id)
            .await
            .err()
            .map(|e| e.code),
        Some(ErrorCode::NotFound)
    );
}

/// The services a daemon offers, and the forms that add them, are its answer
/// rather than a list a client keeps in step by hand.
#[tokio::test]
async fn services_and_their_forms_agree_across_transports() {
    let pair = spawn_pair().await;

    let local = pair.local.services().await.expect("local services");
    let wire = pair.wire.services().await.expect("wire services");
    assert_eq!(local, wire);
    assert!(
        local.iter().any(|service| service.id == "jellyfin"),
        "the services are named by their stable ids: {local:?}"
    );
    let youtube = local
        .iter()
        .find(|service| service.id == "ytmusic")
        .expect("youtube music is offered");
    assert!(
        youtube
            .fields
            .iter()
            .any(|field| matches!(field.kind, api::FieldKind::Radio { .. })),
        "its form asks how to sign in: {:?}",
        youtube.fields
    );
    assert!(
        youtube.fields.iter().any(|field| field.show_when.is_some()),
        "and hides the browser picker unless it is signing in"
    );
}

/// Checking a draft is what tells a client whether it may be saved, so both
/// transports must give the same verdict.
#[tokio::test]
async fn a_draft_is_checked_identically_across_transports() {
    let pair = spawn_pair().await;

    let missing = api::SourceDraft {
        name: String::new(),
        service: "spotify".into(),
        ..Default::default()
    };
    let local = pair
        .local
        .check_source_draft(missing.clone())
        .await
        .expect("local check");
    let wire = pair
        .wire
        .check_source_draft(missing)
        .await
        .expect("wire check");
    assert_eq!(local, wire);
    assert_eq!(local.sign_in, api::SignInKind::Browser);
    assert!(
        local
            .problems
            .iter()
            .any(|problem| problem.field.as_deref() == Some("client_id")),
        "a Spotify server needs its client id: {local:?}"
    );

    let good = api::SourceDraft {
        name: "Home".into(),
        service: "jellyfin".into(),
        values: vec![api::FieldValue::new("url", "https://jelly.example")],
        ..Default::default()
    };
    let check = pair.wire.check_source_draft(good).await.expect("check");
    assert!(
        check.problems.is_empty(),
        "nothing wrong with it: {check:?}"
    );
    assert_eq!(check.sign_in, api::SignInKind::Password);

    // A service this daemon does not have is invalid input, not a panic.
    let unknown = api::SourceDraft {
        name: "Home".into(),
        service: "not-a-service".into(),
        ..Default::default()
    };
    assert_eq!(
        pair.local
            .check_source_draft(unknown.clone())
            .await
            .err()
            .map(|error| error.code),
        pair.wire
            .check_source_draft(unknown)
            .await
            .err()
            .map(|error| error.code),
    );
}

/// A source's own options round-trip, and answering one key leaves the others
/// alone.
#[tokio::test]
async fn source_settings_round_trip_without_touching_what_was_not_answered() {
    let pair = spawn_pair().await;

    let added = pair
        .wire
        .upsert_source(api::SourceDraft {
            name: "Music".into(),
            service: "applemusic".into(),
            values: vec![
                api::FieldValue::new("storefront", "gb"),
                api::FieldValue::new("language", "en"),
                api::FieldValue::new("browser", "chrome"),
            ],
            ..Default::default()
        })
        .await
        .expect("add server");
    assert_eq!(
        api::spec_value(&added.settings, "storefront"),
        Some("gb"),
        "the form's answers come back as its settings: {:?}",
        added.settings
    );

    let updated = pair
        .local
        .set_source_settings(
            added.id.clone(),
            vec![api::FieldValue::new("storefront", "jp")],
        )
        .await
        .expect("set settings");
    assert_eq!(api::spec_value(&updated.settings, "storefront"), Some("jp"));
    assert_eq!(
        api::spec_value(&updated.settings, "language"),
        Some("en"),
        "a key nobody answered is left alone: {:?}",
        updated.settings
    );
    assert_eq!(
        pair.wire.sources().await.expect("wire sources"),
        pair.local.sources().await.expect("local sources"),
    );
}

/// The downloader is a subprocess the daemon owns. Whether it is installed is
/// a daemon fact, so both transports report its absence the same way rather
/// than one of them guessing.
#[tokio::test]
async fn the_downloader_reports_its_preconditions_identically() {
    let pair = spawn_pair().await;

    let formats = pair.local.download_formats().await.expect("formats");
    assert_eq!(
        formats,
        pair.wire.download_formats().await.expect("wire formats"),
    );
    let first = formats.first().expect("at least one format").value.clone();

    assert_eq!(
        pair.local
            .download_url(String::new(), first.clone())
            .await
            .err()
            .map(|error| error.code),
        Some(ErrorCode::InvalidInput),
        "a request with no URL is rejected before anything is spawned"
    );
    assert_eq!(
        pair.local
            .download_url(String::new(), first.clone())
            .await
            .err()
            .map(|e| e.code),
        pair.wire
            .download_url(String::new(), first.clone())
            .await
            .err()
            .map(|e| e.code),
    );
    assert_eq!(
        pair.local
            .download_url("https://example.com/watch".into(), "not-a-format".into())
            .await
            .err()
            .map(|error| error.code),
        Some(ErrorCode::InvalidInput),
        "a format the daemon does not offer is refused"
    );

    // With a URL, the answer depends on whether the tools are installed --
    // whatever it is, it must be the same on both sides.
    let local = pair
        .local
        .download_url("https://example.com/watch".into(), first.clone())
        .await;
    let wire = pair
        .wire
        .download_url("https://example.com/watch".into(), first)
        .await;
    assert_eq!(
        local.is_ok(),
        wire.is_ok(),
        "local {local:?} vs wire {wire:?}"
    );
}

/// The downloader's options are published rather than known by a client, so
/// both transports must describe them the same way and a write must stick.
#[tokio::test]
async fn downloader_settings_round_trip_across_transports() {
    let pair = spawn_pair().await;

    let local = pair.local.downloader_settings().await.expect("settings");
    assert_eq!(
        local,
        pair.wire
            .downloader_settings()
            .await
            .expect("wire settings"),
    );
    assert!(
        api::spec_value(&local, "output_dir").is_some(),
        "the download location is one of them: {local:?}"
    );

    let updated = pair
        .wire
        .set_downloader_settings(vec![api::FieldValue::new("embed_thumbnail", "false")])
        .await
        .expect("set settings");
    assert_eq!(api::spec_value(&updated, "embed_thumbnail"), Some("false"));
    assert_eq!(
        api::spec_value(&updated, "embed_metadata"),
        api::spec_value(&local, "embed_metadata"),
        "a key nobody answered is left alone",
    );
    assert_eq!(
        pair.local.downloader_settings().await.expect("settings"),
        updated,
        "and the write is what both sides now read",
    );

    assert_eq!(
        pair.local.downloader_history().await.expect("history"),
        pair.wire.downloader_history().await.expect("wire history"),
    );
    pair.wire
        .clear_downloader_history()
        .await
        .expect("clear history");
}

/// What is configured per account is published the same way a service is, and
/// answering a field never echoes the secret back.
#[tokio::test]
async fn integrations_agree_across_transports_and_carry_no_secret() {
    let pair = spawn_pair().await;

    let local = pair.local.integrations().await.expect("local integrations");
    assert_eq!(local, pair.wire.integrations().await.expect("wire"));
    let lastfm = local
        .iter()
        .find(|integration| integration.id == "lastfm")
        .expect("last.fm is offered");
    assert!(
        !lastfm.configured,
        "nothing is connected in a fresh library"
    );
    assert_eq!(lastfm.connect, api::ConnectKind::WebSignIn);
    assert!(
        lastfm
            .fields
            .iter()
            .all(|field| matches!(field.kind, api::FieldKind::Secret) && field.value.is_none()),
        "its fields are secrets, and a secret is never sent out: {:?}",
        lastfm.fields
    );

    let updated = pair
        .wire
        .set_integration_settings(
            "lastfm".to_string(),
            vec![api::FieldValue::new("api_key", "a-key-nobody-should-see")],
        )
        .await
        .expect("set settings");
    let rendered = format!("{updated:?}");
    assert!(
        !rendered.contains("a-key-nobody-should-see"),
        "no response may carry the secret: {rendered}"
    );
    let settings = format!("{:?}", pair.wire.config().await.expect("wire view"));
    assert!(
        !settings.contains("a-key-nobody-should-see"),
        "nor may the settings a client reads: {settings}"
    );

    assert_eq!(
        pair.local
            .set_integration_settings("nope".to_string(), Vec::new())
            .await
            .err()
            .map(|error| error.code),
        pair.wire
            .set_integration_settings("nope".to_string(), Vec::new())
            .await
            .err()
            .map(|error| error.code),
        "an integration this daemon does not have is refused the same way",
    );
}

/// The profile list is read from the daemon's machine, and connecting takes
/// only an id from it, never a path the daemon did not list.
#[tokio::test]
async fn browser_connect_agrees_across_transports() {
    let pair = spawn_pair().await;

    assert_eq!(
        pair.local.browser_profiles().await.expect("local profiles"),
        pair.wire.browser_profiles().await.expect("wire profiles"),
    );
    let listenbrainz = pair
        .wire
        .integrations()
        .await
        .expect("integrations")
        .into_iter()
        .find(|integration| integration.id == "listenbrainz")
        .expect("listenbrainz is offered");
    assert_eq!(listenbrainz.connect, api::ConnectKind::BrowserProfile);

    for (id, code) in [
        ("listenbrainz", api::ErrorCode::NotFound),
        ("lastfm", api::ErrorCode::Unsupported),
    ] {
        let local = pair
            .local
            .connect_integration_from_browser(id.to_string(), "/not/a/profile".to_string())
            .await
            .err()
            .map(|error| error.code);
        assert_eq!(local, Some(code), "{id}");
        assert_eq!(
            local,
            pair.wire
                .connect_integration_from_browser(id.to_string(), "/not/a/profile".to_string())
                .await
                .err()
                .map(|error| error.code),
            "{id} is refused the same way on the wire",
        );
    }
}

/// The download overlay reads per-item state, so a client on the socket has to
/// see the same list the in-process one does -- including the failures, which
/// is what a batch against unreachable files produces.
#[tokio::test]
async fn download_statuses_agree_across_transports() {
    let pair = spawn_pair().await;

    assert_eq!(
        pair.local
            .download_statuses()
            .await
            .expect("local statuses"),
        pair.wire.download_statuses().await.expect("wire statuses"),
        "an idle daemon reports the same empty list on both transports"
    );

    // The seeded rows are local paths that do not exist, so every item fails --
    // which is the interesting case: a failed item is still reported, and
    // reported the same way on both sides.
    let keys = vec![
        "/lib/seed-0.flac".to_string(),
        "/lib/seed-1.flac".to_string(),
    ];
    let job = pair.wire.download(keys).await.expect("the batch starts");

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let running =
                pair.jobs.list().into_iter().any(|status| {
                    status.id == job.job_id && status.state == api::JobState::Running
                });
            if !running {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the batch finished");

    let local = pair
        .local
        .download_statuses()
        .await
        .expect("local statuses");
    let wire = pair.wire.download_statuses().await.expect("wire statuses");
    assert_eq!(local, wire, "local {local:?} vs wire {wire:?}");

    assert_eq!(
        pair.local.downloads().await.expect("local downloads"),
        pair.wire.downloads().await.expect("wire downloads"),
        "nothing landed offline, and both say so"
    );
}

#[tokio::test]
async fn both_transports_shake_hands_on_this_revision() {
    let pair = spawn_pair().await;

    for handshake in [pair.local.handshake().await, pair.wire.handshake().await] {
        match handshake.expect("status") {
            api::Handshake::Ready(status) => assert_eq!(status.proto_revision, api::WIRE_REVISION),
            mismatched => panic!("{mismatched:?}"),
        }
    }
}

#[tokio::test]
async fn version_switches_and_their_errors_map_identically() {
    let pair = spawn_pair().await;
    pair.wire
        .set_queue(replace(&["ytm-song:a", "/b.wav"]))
        .await
        .expect("set queue");
    wait_state(&pair.local, "committed", |state| {
        matches!(state.intent, Intent::Committed { .. })
    })
    .await;
    let local = pair.local.player_state().await.expect("local state");
    let wire = pair.wire.player_state().await.expect("wire state");
    let song = local.track.clone().expect("the song plays");
    assert_eq!(song.version(), Some(api::TrackVersion::Song));
    assert_eq!(wire.track, local.track, "the pairing crosses the wire");

    pair.wire
        .player_command(PlayerCommand::SetVersion {
            version: api::TrackVersion::Video,
        })
        .await
        .expect("switch over the wire");
    let state = wait_state(&pair.local, "the video committed", |state| {
        state
            .track
            .as_ref()
            .is_some_and(|t| t.key == "ytm-song:a-video")
            && matches!(state.intent, Intent::Committed { .. })
    })
    .await;
    let video = state.track.expect("the video plays");
    assert_eq!(video.version(), Some(api::TrackVersion::Video));
    assert_eq!(
        video.counterpart.as_ref().map(|other| other.key.as_str()),
        Some("ytm-song:a")
    );
    assert_eq!(state.queue.length, 2);
    let queue = pair
        .wire
        .queue_snapshot()
        .await
        .expect("queue over the wire");
    assert_eq!(queue.items[0].key, "ytm-song:a-video");
    assert_eq!(queue.items[1].key, "/b.wav");

    // This pair serves no pictures, and says so the same way on both sides.
    let request = api::VideoRequest {
        key: "ytm-song:a-video".into(),
        start: 0,
        length: Some(1024),
    };
    let local_err = pair
        .local
        .video(request.clone())
        .await
        .expect_err("no video locally");
    let wire_err = pair
        .wire
        .video(request)
        .await
        .expect_err("no video over the wire");
    assert_eq!(local_err.code, ErrorCode::Unsupported);
    assert_eq!(
        (wire_err.code, wire_err.message),
        (local_err.code, local_err.message)
    );

    pair.wire
        .player_command(PlayerCommand::Next)
        .await
        .expect("next");
    wait_state(&pair.local, "the plain track", |state| {
        state.track.as_ref().is_some_and(|t| t.key == "/b.wav")
            && matches!(state.intent, Intent::Committed { .. })
    })
    .await;
    let switch = PlayerCommand::SetVersion {
        version: api::TrackVersion::Video,
    };
    let local_err = pair
        .local
        .player_command(switch)
        .await
        .expect_err("no pair locally");
    let wire_err = pair
        .wire
        .player_command(switch)
        .await
        .expect_err("no pair over the wire");
    assert_eq!(local_err.code, ErrorCode::InvalidInput);
    assert_eq!(
        (wire_err.code, wire_err.message),
        (local_err.code, local_err.message)
    );
}

/// "Playing from" is the daemon's: whoever built the queue, every transport reads the same origin, and a raw track list has none.
#[tokio::test]
async fn a_queue_origin_is_the_same_on_both_transports() {
    let pair = spawn_pair().await;
    let from = |mode, context| SetQueueRequest {
        mode,
        context,
        start_index: None,
        shuffle: None,
    };
    let origin = async |api: &LocalApi| api.player_state().await.expect("state").queue.context;

    let album = QueueContext::Album { id: "a".into() };
    pair.wire
        .set_queue(from(QueueMode::Replace, album.clone()))
        .await
        .expect("set queue over the wire");
    assert_eq!(origin(&pair.local).await, Some(album.clone()));
    assert_eq!(
        pair.wire.player_state().await.expect("state").queue.context,
        Some(album.clone())
    );
    let local = pair.local.queue_snapshot().await.expect("local snapshot");
    let wire = pair.wire.queue_snapshot().await.expect("wire snapshot");
    assert_eq!(local.context, Some(album.clone()));
    assert_eq!(local, wire);

    // Edits and appended tracks leave it alone, whichever transport made them.
    pair.wire
        .set_queue(from(
            QueueMode::Append,
            QueueContext::Tracks {
                keys: vec!["/lib/extra.flac".into()],
            },
        ))
        .await
        .expect("append over the wire");
    pair.local
        .queue_edit(QueueEdit::Move { from: 2, to: 0 })
        .await
        .expect("move");
    assert_eq!(
        pair.wire.queue_snapshot().await.expect("snapshot").context,
        Some(album)
    );

    let playlist = QueueContext::Playlist { id: "p".into() };
    pair.local
        .set_queue(from(QueueMode::Replace, playlist.clone()))
        .await
        .expect("set queue");
    assert_eq!(
        pair.wire.queue_snapshot().await.expect("snapshot").context,
        Some(playlist)
    );

    let radio = QueueContext::Radio {
        station_id: "s".into(),
        stream_id: "st".into(),
    };
    pair.wire
        .set_queue(from(QueueMode::Replace, radio.clone()))
        .await
        .expect("set queue");
    assert_eq!(
        pair.local.queue_snapshot().await.expect("snapshot").context,
        Some(radio)
    );

    pair.wire
        .set_queue(replace(&["/lib/raw-0", "/lib/raw-1"]))
        .await
        .expect("set queue");
    assert_eq!(origin(&pair.local).await, None);
    wait_state(&pair.local, "committed", |state| {
        matches!(state.intent, Intent::Committed { .. })
    })
    .await;
    let local = normalized(pair.local.player_state().await.expect("state"));
    assert_eq!(
        local,
        normalized(pair.wire.player_state().await.expect("state"))
    );
}

/// Mute is the daemon's: either transport sets it, both read it, and the level to return to is untouched.
#[tokio::test]
async fn mute_is_the_same_on_both_transports() {
    use futures_util::StreamExt;
    let pair = spawn_pair().await;
    let level = pair.local.player_state().await.expect("state").volume;
    assert!(!pair.wire.player_state().await.expect("state").muted);

    pair.wire
        .player_command(PlayerCommand::SetMuted { muted: true })
        .await
        .expect("mute over the wire");
    for state in [
        pair.local.player_state().await.expect("local"),
        pair.wire.player_state().await.expect("wire"),
    ] {
        assert!(state.muted);
        assert_eq!(state.volume, level);
    }
    assert_eq!(
        normalized(pair.local.player_state().await.expect("local")),
        normalized(pair.wire.player_state().await.expect("wire"))
    );

    // Setting a level unmutes, whichever transport asks.
    pair.local
        .player_command(PlayerCommand::SetVolume { volume: 0.25 })
        .await
        .expect("set volume");
    let state = pair.wire.player_state().await.expect("wire");
    assert!((state.muted, state.volume) == (false, 0.25));

    pair.local
        .set_queue(SetQueueRequest {
            mode: QueueMode::Replace,
            context: QueueContext::Album { id: "a".into() },
            start_index: None,
            shuffle: None,
        })
        .await
        .expect("set queue");
    let mut events = pair.wire.events();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut heard = false;
    while !heard {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for a player_state event with the mute and origin"
        );
        pair.wire
            .player_command(PlayerCommand::SetMuted { muted: true })
            .await
            .expect("mute");
        while let Ok(Some(event)) =
            tokio::time::timeout(Duration::from_millis(500), events.next()).await
        {
            if let ApiEvent::PlayerState(state) = event
                && state.muted
                && state.volume == 0.25
                && state.queue.context.is_some()
            {
                heard = true;
                break;
            }
        }
    }
}

/// The upcoming edits are one daemon command each; both transports must land them on the same queue.
#[tokio::test]
async fn upcoming_edits_agree_across_transports() {
    let pair = spawn_pair().await;
    let keys: Vec<String> = (0..12).map(|i| format!("/lib/u{i:02}.flac")).collect();
    pair.local
        .set_queue(SetQueueRequest {
            mode: QueueMode::Replace,
            context: QueueContext::Tracks { keys: keys.clone() },
            start_index: Some(0),
            shuffle: Some(false),
        })
        .await
        .expect("seed the queue");
    pair.wire
        .queue_edit(QueueEdit::Jump { index: 2 })
        .await
        .expect("jump over the wire");

    let mut events = pair.session.subscribe();
    pair.wire
        .queue_edit(QueueEdit::ShuffleUpcoming)
        .await
        .expect("shuffle over the wire");
    pair.local
        .queue_edit(QueueEdit::ShuffleUpcoming)
        .await
        .expect("shuffle locally");
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut queue_changes = 0;
    while let Ok(event) = events.try_recv() {
        if matches!(event, api::ApiEvent::QueueChanged { .. }) {
            queue_changes += 1;
        }
    }
    assert_eq!(queue_changes, 2, "each edit publishes the queue once");
    let local = pair.local.queue_snapshot().await.expect("local snapshot");
    let wire = pair.wire.queue_snapshot().await.expect("wire snapshot");
    for snapshot in [&local, &wire] {
        let got: Vec<String> = snapshot.items.iter().map(|i| i.key.clone()).collect();
        assert_eq!(
            got[..3],
            keys[..3],
            "history and the playing track stay put"
        );
        assert_eq!(snapshot.position, Some(2));
        let mut rest = got[3..].to_vec();
        rest.sort();
        assert_eq!(rest, keys[3..], "the same tracks, reordered");
    }
    assert_eq!(
        local.items.iter().map(|i| &i.key).collect::<Vec<_>>(),
        wire.items.iter().map(|i| &i.key).collect::<Vec<_>>(),
    );

    pair.wire
        .queue_edit(QueueEdit::ReplaceUpcoming {
            keys: vec!["/lib/x.flac".into(), "/lib/y.flac".into()],
        })
        .await
        .expect("replace over the wire");
    let mut expected = keys[..3].to_vec();
    expected.extend(["/lib/x.flac".to_string(), "/lib/y.flac".to_string()]);
    for snapshot in [
        pair.local.queue_snapshot().await.expect("local snapshot"),
        pair.wire.queue_snapshot().await.expect("wire snapshot"),
    ] {
        let got: Vec<String> = snapshot.items.iter().map(|i| i.key.clone()).collect();
        assert_eq!(got, expected);
    }

    pair.wire
        .queue_edit(QueueEdit::ClearUpcoming)
        .await
        .expect("clear over the wire");
    let snapshot = pair.local.queue_snapshot().await.expect("snapshot");
    assert_eq!(snapshot.items.len(), 3);
    assert_eq!(snapshot.position, Some(2));

    // With nothing upcoming both transports accept the edit and neither publishes.
    let rev = pair.local.player_state().await.expect("state").rev;
    for edit in [
        QueueEdit::ClearUpcoming,
        QueueEdit::ShuffleUpcoming,
        QueueEdit::ReplaceUpcoming { keys: Vec::new() },
    ] {
        let local = pair
            .local
            .queue_edit(edit.clone())
            .await
            .expect("local no-op");
        let wire = pair.wire.queue_edit(edit).await.expect("wire no-op");
        assert_eq!((local.rev, wire.rev), (rev, rev));
    }
}

/// Offline mode is the same daemon state on both transports: entered and left by either, heard by both.
#[tokio::test]
async fn offline_mode_is_set_and_heard_on_both_transports() {
    use futures_util::StreamExt;
    let pair = spawn_pair().await;
    let mut events = pair.local.events();

    let on = pair.wire.set_offline(true).await.expect("enter");
    assert_eq!(on.offline_mode, api::OfflineMode::Requested);
    for api in [&pair.local as &dyn KopuzApi, &pair.wire] {
        assert_eq!(api.network_status().await, Ok(on.clone()));
    }
    let heard = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(event) = events.next().await {
            if let ApiEvent::NetworkStatus(status) = event {
                return status;
            }
        }
        panic!("the event stream ended");
    })
    .await
    .expect("published");
    assert_eq!(heard, on);

    let off = pair.local.set_offline(false).await.expect("leave");
    assert_eq!(off, api::NetworkStatus::default());
    assert_eq!(pair.wire.network_status().await, Ok(off));
}

/// The offline cache's status and its clearing answer alike on both transports, and neither touches a download.
#[tokio::test]
async fn the_offline_cache_answers_alike_on_both_transports() {
    let pair = spawn_pair().await;
    let local = pair.local.offline_cache_status().await.expect("local");
    assert_eq!(pair.wire.offline_cache_status().await, Ok(local));
    assert_eq!(local.used_bytes, 0);
    assert_eq!(local.max_bytes, config::DEFAULT_OFFLINE_CACHE_MAX_BYTES);

    pair.wire.clear_offline_cache().await.expect("wire clear");
    pair.local.clear_offline_cache().await.expect("local clear");
    assert_eq!(pair.wire.offline_cache_status().await, Ok(local));
}

/// The `offline` filter rides the wire and keeps to rows with a copy, a downloaded one included.
#[tokio::test]
async fn the_offline_filter_agrees_on_both_transports() {
    let pair = spawn_pair().await;
    let copy = pair._dir.path().join("seed-0.flac");
    std::fs::write(&copy, b"audio").expect("a copy");
    pair.database
        .set_offline_track("/lib/seed-0.flac", Some(&copy.to_string_lossy()))
        .await
        .expect("registered");
    let everything = Page {
        offset: 0,
        limit: 100,
    };
    for offline in [Some(true), Some(false)] {
        let filter = TrackFilter {
            offline,
            ..Default::default()
        };
        let local = pair
            .local
            .tracks(filter.clone(), everything)
            .await
            .expect("local");
        let wire = pair.wire.tracks(filter, everything).await.expect("wire");
        assert_eq!(local, wire, "{offline:?}");
        let keys: Vec<&str> = wire.items.iter().map(|row| row.key.as_str()).collect();
        match offline {
            Some(true) => assert_eq!(keys, ["/lib/seed-0.flac"]),
            _ => assert_eq!(keys, ["/lib/seed-1.flac"]),
        }
    }
    let albums = api::AlbumQuery {
        offline: Some(true),
        ..Default::default()
    };
    assert_eq!(
        pair.local.albums(albums.clone(), everything).await,
        pair.wire.albums(albums, everything).await
    );
    let artists = api::ArtistQuery {
        offline: Some(true),
        ..Default::default()
    };
    assert_eq!(
        pair.local.artists(artists.clone(), everything).await,
        pair.wire.artists(artists, everything).await
    );
}
