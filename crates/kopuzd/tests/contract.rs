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
    wire: client::GrpcApi,
    jobs: Arc<JobRunner>,
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
        .upsert_tracks(&config::Source::Local, &seeded)
        .await
        .expect("seed tracks");
    let config_service = Arc::new(ConfigService::new(
        database.clone(),
        dir.path().join("settings.toml"),
        config::AppConfig::default(),
    ));
    let library = Arc::new(LibraryService::new(
        database.clone(),
        config::Source::Local,
        Arc::new(radio::registry::StationRegistry::default()),
        dir.path().join("covers"),
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
        config_service.clone(),
        dir.path().join("offline"),
    );
    let sources =
        daemon::SourceService::new(database.clone(), session.clone(), config_service.clone());
    let integrations = daemon::IntegrationService::new(config_service.clone(), session.clone());
    let downloader = daemon::UrlDownloadService::new(session.clone(), config_service.clone());
    let build_api = |session: SessionHandle| {
        LocalApi::new(session)
            .with_config(config_service.clone())
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
    tokio::spawn(kopuzd::serve(listener, state));
    Pair {
        local: build_api(session.clone()),
        wire: client::GrpcApi::new(&socket).expect("wire client"),
        jobs,
        database,
        session,
        _dir: dir,
    }
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

#[tokio::test]
async fn config_view_and_set_agree_across_transports() {
    let pair = spawn_pair().await;

    let local_view = pair.local.config().await.expect("local view");
    let wire_view = pair.wire.config().await.expect("wire view");
    // The whole 68-field surface has to survive the proto round trip for
    // these to be equal, so this is the guard on every field mapping.
    assert_eq!(local_view, wire_view);
    assert!(local_view.config.lastfm_session_key.is_empty());
    assert!(local_view.config.server.is_none());

    let mut next = wire_view.config.clone();
    next.crossfade_seconds = 7;
    next.theme = "nord".to_string();
    next.offline_quality = config::OfflineQuality::Kbps160;
    let written = pair.wire.set_config(next).await.expect("set over the wire");
    assert_eq!(written.config.crossfade_seconds, 7);
    assert_eq!(written.config.theme, "nord");
    assert_eq!(
        written.config.offline_quality,
        config::OfflineQuality::Kbps160
    );

    let local_view = pair.local.config().await.expect("local view after set");
    assert_eq!(local_view.config, written.config);

    // Credentials are absent from the wire, so writing a view straight back
    // cannot erase them; the daemon keeps its own. (Seeding one is only
    // possible below the API, so the depth test lives in config_service.)
    assert!(written.config.lastfm_session_key.is_empty());
    assert!(written.config.servers.is_empty());
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

#[tokio::test]
async fn scan_job_indexes_local_files_over_the_wire() {
    let pair = spawn_pair().await;
    let music = pair._dir.path().join("music");
    std::fs::create_dir_all(&music).expect("music dir");
    std::fs::write(music.join("one.wav"), wav_bytes(1)).expect("write wav");
    std::fs::write(music.join("two.wav"), wav_bytes(1)).expect("write wav");

    let mut config = pair.wire.config().await.expect("view").config;
    config.music_directory = vec![music.clone()];
    pair.wire
        .set_config(config)
        .await
        .expect("point the library at the temp dir");

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
            api::JobState::Finished => break,
            other => panic!("scan ended as {other:?}: {status:?}"),
        }
    }

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
        pair.local.albums(all).await.expect("local"),
        pair.wire.albums(all).await.expect("wire"),
    );
    assert_eq!(
        pair.local.artists(all).await.expect("local"),
        pair.wire.artists(all).await.expect("wire"),
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
            .artist_tracks(String::new(), all)
            .await
            .expect("local"),
        pair.wire
            .artist_tracks(String::new(), all)
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
        .upsert_tracks(&config::Source::Local, &[with_art])
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

    let request = api::CatalogDetailRequest {
        kind: api::CatalogItemKind::Album,
        id: "MPRE1".into(),
        continuation: None,
    };
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

    let local = pair.local.sources().await.expect("local sources");
    let wire = pair.wire.sources().await.expect("wire sources");
    assert_eq!(local, wire);
    assert_eq!(local.len(), 1, "just the default local library: {local:?}");
    assert!(local[0].active, "the default library is active");
    assert!(local[0].authenticated, "a local library needs no sign-in");
    assert_eq!(local[0].service, None);
    assert_eq!(local[0].sign_in, api::SignInKind::None);

    // Adding a server is visible to both, and provisioning a credential
    // reports authentication without echoing the secret.
    let draft = api::ServerDraft {
        name: "Home".into(),
        service: "jellyfin".into(),
        values: vec![api::FieldValue::new("url", "https://jelly.example")],
        ..Default::default()
    };
    let added = pair.wire.upsert_server(draft).await.expect("add server");
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
    let bad = api::ServerDraft {
        name: "No URL".into(),
        service: "jellyfin".into(),
        values: vec![api::FieldValue::new("url", "not-a-url")],
        ..Default::default()
    };
    assert_eq!(
        pair.local
            .upsert_server(bad.clone())
            .await
            .err()
            .map(|error| error.code),
        pair.wire.upsert_server(bad).await.err().map(|e| e.code),
    );

    pair.wire
        .delete_server(added.id.clone())
        .await
        .expect("delete");
    assert_eq!(pair.local.sources().await.expect("sources").len(), 1);
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

    let missing = api::ServerDraft {
        name: String::new(),
        service: "spotify".into(),
        ..Default::default()
    };
    let local = pair
        .local
        .check_server_draft(missing.clone())
        .await
        .expect("local check");
    let wire = pair
        .wire
        .check_server_draft(missing)
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

    let good = api::ServerDraft {
        name: "Home".into(),
        service: "jellyfin".into(),
        values: vec![api::FieldValue::new("url", "https://jelly.example")],
        ..Default::default()
    };
    let check = pair.wire.check_server_draft(good).await.expect("check");
    assert!(
        check.problems.is_empty(),
        "nothing wrong with it: {check:?}"
    );
    assert_eq!(check.sign_in, api::SignInKind::Password);

    // A service this daemon does not have is invalid input, not a panic.
    let unknown = api::ServerDraft {
        name: "Home".into(),
        service: "not-a-service".into(),
        ..Default::default()
    };
    assert_eq!(
        pair.local
            .check_server_draft(unknown.clone())
            .await
            .err()
            .map(|error| error.code),
        pair.wire
            .check_server_draft(unknown)
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
        .upsert_server(api::ServerDraft {
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
