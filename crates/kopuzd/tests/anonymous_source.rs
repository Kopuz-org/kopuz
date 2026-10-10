//! An anonymous YouTube Music source has no credential row at all, and must
//! still build the real backend rather than the offline stand-in, over the
//! socket exactly as in-process.

use std::sync::Arc;
use std::time::Instant;

use api::{ApiError, QueueContext, SourceApi};
use daemon::session::FactoryOverride;
use daemon::{ConfigService, LocalApi, QueueMaterializer, SessionHandle};
use player::engine::NullSink;
use player::player::Player;
use reader::Track;

struct NoLibrary;

#[async_trait::async_trait]
impl QueueMaterializer for NoLibrary {
    async fn materialize(&self, _: &QueueContext) -> Result<Vec<Track>, ApiError> {
        Err(ApiError::unsupported("no library in this test"))
    }
}

#[tokio::test]
async fn an_anonymous_youtube_music_source_is_usable_without_a_credential() {
    let dir = tempfile::tempdir().expect("tempdir");
    let database = db::init(&dir.path().join("anon.db"))
        .await
        .expect("db init");
    let config = Arc::new(ConfigService::new(
        database.clone(),
        dir.path().join("settings.toml"),
        config::AppConfig::default(),
    ));
    let player = Player::try_with_sink(Box::new(NullSink::new())).expect("headless player starts");
    let provider: FactoryOverride = Arc::new(|_| None);
    let session = SessionHandle::spawn_with_factory(
        Arc::new(NoLibrary),
        player,
        daemon::PlaybackServices::default(),
        provider,
    );
    config.attach_session(session.clone());
    let sources = daemon::SourceService::new(database.clone(), session.clone(), config.clone());
    let local = Arc::new(
        LocalApi::new(session.clone())
            .with_config(config.clone())
            .with_sources(sources),
    );
    let state = Arc::new(kopuzd::GrpcState {
        api: local.clone(),
        artwork: None,
        session,
        started: Instant::now(),
    });
    let socket = dir.path().join("kopuzd.sock");
    let listener = kopuzd::bind_socket(&socket).expect("bind socket");
    tokio::spawn(kopuzd::serve(listener, state));
    let wire = client::GrpcApi::new(&socket).expect("wire client");

    let added = wire
        .upsert_source(api::SourceDraft {
            name: "YouTube Music".into(),
            service: "ytmusic".into(),
            values: vec![api::FieldValue::new("auth_method", "anonymous")],
            ..Default::default()
        })
        .await
        .expect("add an anonymous source");
    assert!(added.anonymous);
    assert!(added.authenticated, "anonymous needs no sign-in");
    assert_eq!(added.sign_in, api::SignInKind::None);

    let switched = wire
        .switch_source(added.id.clone())
        .await
        .expect("switch to it");
    for (transport, info) in [
        ("wire", switched),
        (
            "local",
            SourceApi::sources(local.as_ref())
                .await
                .expect("local sources")
                .into_iter()
                .find(|source| source.id == added.id)
                .expect("listed"),
        ),
    ] {
        assert!(info.active, "{transport}: it is the active source");
        assert!(
            info.capabilities.discover && info.capabilities.track_radio,
            "{transport}: the YouTube Music backend is built, not the offline stand-in: {:?}",
            info.capabilities
        );
    }
}
