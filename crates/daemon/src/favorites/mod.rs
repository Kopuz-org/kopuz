//! FavoritesService: the optimistic toggle and the background reconciler,
//! ported from `hooks/src/favorites.rs` and `hooks/src/use_sync_task.rs`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use api::{ApiError, ApiEvent, ErrorCode, FavoritesView, JobKind, JobRef, Table};
use server::sync::{SyncError, SyncReason, reconcile_favorites};
use tokio::sync::{Mutex, Notify};

use crate::jobs::{JobRunner, Trigger};
use crate::session::SessionHandle;
use crate::sources::source_error;

mod pull;

const NUDGE_DEBOUNCE: Duration = Duration::from_secs(2);
const BACKOFF_CAP_SECS: u64 = 30 * 60;

/// An expired session says nothing here: the caller recovers it and pushes again.
fn push_rejected_notice(error: &server::source::SourceError) -> Option<ApiEvent> {
    if matches!(error, server::source::SourceError::Auth) {
        return None;
    }
    Some(ApiEvent::Notice {
        level: api::NoticeLevel::Error,
        code: "favorite_push_rejected".to_string(),
        message: Some(error.to_string()),
    })
}

pub struct FavoritesService {
    db: db::Db,
    session: SessionHandle,
    generation: AtomicU64,
    nudge: Notify,
    mutation_nudge: AtomicBool,
    /// Held for a whole sync, so a second caller waits on the one running and takes its outcome.
    flight: Mutex<Flight>,
    /// Syncs started so far; the one running is this number.
    started: AtomicU64,
}

/// How much a sync imports after it pushes what is pending.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Import {
    /// Always: someone asked for a favorites sync.
    Always,
    /// Unless a favorites sync finished within the sync interval.
    UnlessRecent,
    /// Only what is missing (nothing imported yet, an instant source's list gone stale), and only after the push went through.
    IfMissing,
}

/// The last sync to finish: which it was, whether it imported, and what it answered.
struct Flight {
    finished: u64,
    imported: bool,
    outcome: Result<(), ApiError>,
}

impl FavoritesService {
    pub fn new(db: db::Db, session: SessionHandle) -> Arc<Self> {
        Arc::new(Self {
            db,
            session,
            generation: AtomicU64::new(0),
            nudge: Notify::new(),
            mutation_nudge: AtomicBool::new(false),
            flight: Mutex::new(Flight {
                finished: 0,
                imported: false,
                outcome: Ok(()),
            }),
            started: AtomicU64::new(0),
        })
    }

    fn active_source(&self) -> server::source::ActiveSource {
        Arc::from(server::source::active(
            self.db.clone(),
            &self.session.sources(),
        ))
    }

    fn bump(&self, table: Table) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.session.invalidate(table);
    }

    pub async fn list(&self) -> Result<FavoritesView, ApiError> {
        let generation = self.generation.load(Ordering::Relaxed);
        let refs = self
            .active_source()
            .favorites()
            .await
            .map_err(source_error)?;
        Ok(FavoritesView { refs, generation })
    }

    /// The track behind a key. Live search results are not in the DB yet; the
    /// materializer resolves them from the library's transient cache so acting
    /// on one still works (`record_favorite` then upserts the track like the
    /// old direct path).
    async fn resolve_track(&self, key: &str) -> Result<reader::Track, ApiError> {
        let active = self.session.sources().active.clone();
        match self
            .db
            .tracks_by_keys(&active, &[key.to_string()])
            .await
            .map_err(|error| ApiError::internal(format!("database error: {error}")))?
            .into_iter()
            .next()
        {
            Some(track) => Ok(track),
            None => self.session.materialize_track(key.to_string()).await,
        }
    }

    /// Optimistic set, matching the hooks toggle: local write reflected
    /// immediately, remote push follows; a rejected push reverts the local
    /// state, emits a notice, and surfaces the error to the caller.
    pub async fn set(&self, key: &str, favorite: bool) -> Result<(), ApiError> {
        if key.trim().is_empty() {
            return Err(ApiError::invalid_input("empty favorite key"));
        }
        let source = self.active_source();
        let track = self.resolve_track(key).await?;

        if source.is_favorite(key).await == favorite {
            return Ok(());
        }
        source
            .record_favorite(&track, favorite)
            .await
            .map_err(source_error)?;
        self.bump(Table::Favorites);
        self.bump(Table::Tracks);

        if let Err(error) = source.push_favorite(key, favorite).await {
            tracing::warn!(%error, track = %track.id.uid(), "favorite push rejected; reverting");
            let _ = source.record_favorite(&track, !favorite).await;
            self.bump(Table::Favorites);
            self.bump(Table::Tracks);
            if let Some(notice) = push_rejected_notice(&error) {
                self.session.emit_event(notice);
            }
            return Err(source_error(error));
        }
        self.mutation_nudge.store(true, Ordering::Relaxed);
        self.nudge.notify_one();
        Ok(())
    }

    /// Tell the source to stop recommending a track. Not a favorite, but it
    /// lives here because the two are one setting on the remote: YouTube takes
    /// a dislike by clearing whatever like it held, so the local heart has to
    /// go with it or it would claim a like the remote no longer has. Pessimistic
    /// on purpose -- nothing local changes until the remote has taken it.
    pub async fn dont_recommend(&self, key: &str) -> Result<(), ApiError> {
        if key.trim().is_empty() {
            return Err(ApiError::invalid_input("empty track key"));
        }
        let source = self.active_source();
        source.dont_recommend(key).await.map_err(source_error)?;
        if source.is_favorite(key).await {
            let track = self.resolve_track(key).await?;
            source
                .record_favorite(&track, false)
                .await
                .map_err(source_error)?;
            self.bump(Table::Favorites);
            self.bump(Table::Tracks);
        }
        Ok(())
    }

    /// A song's like is its favorite, so the heart follows once the source answers.
    pub async fn rate(&self, item_ref: &str, rating: api::Rating) -> Result<(), ApiError> {
        use server::ytmusic::discover::Rating;
        if item_ref.trim().is_empty() {
            return Err(ApiError::invalid_input("nothing to rate"));
        }
        let source = self.active_source();
        if !source.capabilities().library_actions.rate {
            return Err(ApiError::unsupported("rating"));
        }
        let remote = match rating {
            api::Rating::None => Rating::Indifferent,
            api::Rating::Like => Rating::Like,
            api::Rating::Dislike => Rating::Dislike,
        };
        source.rate(item_ref, remote).await.map_err(source_error)?;

        let Ok(track) = self.resolve_track(item_ref).await else {
            // An album or a playlist: rating one saves it or takes it out.
            self.session.invalidate(Table::Albums);
            self.session.invalidate(Table::Playlists);
            return Ok(());
        };
        let favorite = rating == api::Rating::Like;
        if source.is_favorite(item_ref).await != favorite {
            source
                .record_favorite(&track, favorite)
                .await
                .map_err(source_error)?;
            self.bump(Table::Favorites);
            self.bump(Table::Tracks);
        }
        Ok(())
    }

    /// Ask the reconciler to run soon without the after-mutation marker
    /// (the app window regained focus).
    pub fn nudge_activate(&self) {
        self.nudge.notify_one();
    }

    /// Push what is pending, then import what the remote holds. Both halves
    /// are one job because a caller only ever wants "make these agree", and
    /// pushing after importing would fight the epoch sweep.
    pub fn spawn_sync(
        self: &Arc<Self>,
        runner: &JobRunner,
        trigger: Trigger,
    ) -> Result<JobRef, ApiError> {
        let service = self.clone();
        runner.start_as(JobKind::FavoritesSync, trigger, move |ctx| async move {
            service
                .sync(SyncReason::Manual, Import::Always, Some(&ctx))
                .await
        })
    }

    /// Push pending likes, then import as `import` says; a caller arriving while a sync runs waits for it and takes its outcome, unless it wanted an import that sync did not do.
    pub(crate) async fn sync(
        &self,
        reason: SyncReason,
        import: Import,
        ctx: Option<&crate::jobs::JobCtx>,
    ) -> Result<(), ApiError> {
        let mut flight = match self.flight.try_lock() {
            Ok(flight) => flight,
            Err(_) => {
                let joined = self.started.load(Ordering::SeqCst);
                let flight = self.flight.lock().await;
                if flight.finished >= joined && (flight.imported || import == Import::IfMissing) {
                    return flight.outcome.clone();
                }
                flight
            }
        };
        let run = self.started.fetch_add(1, Ordering::SeqCst) + 1;
        let config = self.session.settings_watch().borrow().clone();
        let source = self.session.sources().active.clone();
        if let Some(ctx) = ctx {
            let phase = match ctx.kind() {
                JobKind::FavoritesSync => "reconciling",
                _ => "reconciling favorites",
            };
            ctx.progress(phase, None, None, None);
        }
        let reconciled = self.reconcile(reason).await;
        let pulled = match import {
            Import::Always => self.pull(ctx, true).await,
            Import::UnlessRecent if self.imported_recently(&config, &source).await => Ok(false),
            Import::UnlessRecent => self.pull(ctx, true).await,
            Import::IfMissing if reconciled.is_err() => Ok(false),
            Import::IfMissing => self.pull(ctx, false).await,
        };
        let imported = matches!(pulled, Ok(true)) && !ctx.is_some_and(|ctx| ctx.cancelled());
        if imported {
            crate::auto_sync::mark_synced(&self.db, JobKind::FavoritesSync, &source).await;
        }
        let outcome = match pulled {
            Err(error) if import != Import::IfMissing => Err(error),
            Err(error) => {
                tracing::debug!(%error, "favorites import skipped");
                reconciled
            }
            Ok(_) => reconciled,
        };
        *flight = Flight {
            finished: run,
            imported,
            outcome: outcome.clone(),
        };
        outcome
    }

    /// Whether the last favorites sync is younger than the sync interval (a day when it is "never").
    async fn imported_recently(&self, config: &config::Settings, source: &config::Source) -> bool {
        let window =
            crate::auto_sync::interval(config).unwrap_or(Duration::from_secs(24 * 60 * 60));
        let Some(at) =
            crate::auto_sync::last_synced(&self.db, JobKind::FavoritesSync, source).await
        else {
            return false;
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or_default();
        now.saturating_sub(at) < window.as_secs()
    }

    async fn reconcile(&self, reason: SyncReason) -> Result<(), ApiError> {
        let sources = self.session.sources();
        let Some(source) = server::source::configured_server(self.db.clone(), &sources) else {
            return Err(ApiError::invalid_input("no server configured"));
        };
        match reconcile_favorites(source.as_ref(), reason).await {
            Ok(report) => {
                if report.pushed_likes + report.pushed_unlikes > 0 || report.did_pull {
                    self.bump(Table::Favorites);
                }
                Ok(())
            }
            Err(SyncError::Expired) => Err(ApiError::new(
                ErrorCode::SourceAuthExpired,
                "credentials expired; sign in again",
            )),
            Err(SyncError::Unreachable(error)) => {
                Err(ApiError::new(ErrorCode::SourceUnreachable, error))
            }
        }
    }

    /// The background reconcile loop: interval per service (5 min YT, 10 min
    /// others) with exponential backoff while unreachable, debounced nudges
    /// after toggles. Fully parked while no server is configured, per the
    /// resource budget.
    pub fn spawn_reconciler(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let service = self.clone();
        tokio::spawn(async move {
            let mut sources_rx = service.session.sources_watch();
            let mut consecutive_failures: u32 = 0;
            // A switch is worth waking for: the new source's favorites were never imported, and an interval's wait leaves the page empty.
            let mut last_source = sources_rx.borrow().active.clone();
            loop {
                let (has_server, base_secs) = {
                    let sources = sources_rx.borrow();
                    let base: u64 = match sources.active_service() {
                        Some(config::MusicService::YtMusic) => 5 * 60,
                        _ => 10 * 60,
                    };
                    (sources.server.is_some(), base)
                };

                if !has_server {
                    tokio::select! {
                        _ = service.nudge.notified() => {}
                        changed = sources_rx.changed() => {
                            if changed.is_err() {
                                return;
                            }
                        }
                    }
                    continue;
                }

                let backoff = base_secs.saturating_mul(1 << consecutive_failures.min(3));
                let interval = Duration::from_secs(backoff.min(BACKOFF_CAP_SECS));
                let nudged = tokio::select! {
                    _ = service.nudge.notified() => true,
                    changed = sources_rx.changed() => {
                        if changed.is_err() {
                            return;
                        }
                        let current = sources_rx.borrow().active.clone();
                        let switched = current != last_source;
                        last_source = current;
                        // Any other change goes back round to re-read the interval rather than costing a network call.
                        if !switched {
                            continue;
                        }
                        true
                    }
                    _ = tokio::time::sleep(interval) => false,
                };
                if nudged {
                    tokio::time::sleep(NUDGE_DEBOUNCE).await;
                }
                let reason = if nudged {
                    if service.mutation_nudge.swap(false, Ordering::Relaxed) {
                        SyncReason::AfterMutation
                    } else {
                        SyncReason::Activate
                    }
                } else {
                    SyncReason::Interval
                };
                // A fresh sign-in fills the favorites page without anyone asking, so what is missing is imported too.
                match service.sync(reason, Import::IfMissing, None).await {
                    Ok(()) => consecutive_failures = 0,
                    Err(error) if error.code == ErrorCode::SourceAuthExpired => {
                        consecutive_failures = consecutive_failures.saturating_add(1);
                        tracing::warn!("favorites sync: credentials expired");
                    }
                    Err(error) => {
                        consecutive_failures = consecutive_failures.saturating_add(1);
                        tracing::debug!(%error, "favorites sync backing off");
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use server::source::SourceError;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn an_expired_session_does_not_announce_a_rejected_push() {
        assert!(push_rejected_notice(&SourceError::Auth).is_none());
        let Some(ApiEvent::Notice { code, .. }) =
            push_rejected_notice(&SourceError::Backend("refused".into()))
        else {
            panic!("a refusal is announced");
        };
        assert_eq!(code, "favorite_push_rejected");
    }

    struct NoTracks;

    #[async_trait::async_trait]
    impl crate::session::QueueMaterializer for NoTracks {
        async fn materialize(&self, _: &api::QueueContext) -> Result<Vec<reader::Track>, ApiError> {
            Ok(Vec::new())
        }
    }

    /// A Subsonic server that answers every call with an empty success, slowly enough for two reconciles to overlap, and counts the requests starting with `counted`.
    async fn subsonic(counted: &'static [u8], hits: Arc<AtomicU64>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let url = format!("http://{}", listener.local_addr().expect("addr"));
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let hits = hits.clone();
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut chunk = [0u8; 1024];
                    while !request.windows(4).any(|end| end == b"\r\n\r\n") {
                        match stream.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(read) => request.extend_from_slice(&chunk[..read]),
                        }
                    }
                    if request.starts_with(counted) {
                        hits.fetch_add(1, Ordering::SeqCst);
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    let body = r#"{"subsonic-response":{"status":"ok","version":"1.16.1"}}"#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        });
        url
    }

    #[tokio::test]
    async fn two_syncs_at_once_push_each_pending_unlike_once() {
        let dir = tempfile::tempdir().expect("tempdir");
        let database = db::init(&dir.path().join("favorites.db"))
            .await
            .expect("db");
        let unstars = Arc::new(AtomicU64::new(0));
        let mut server = config::MusicServer::new_with_service(
            "Navidrome".into(),
            subsonic(b"GET /rest/unstar.view", unstars.clone()).await,
            config::MusicService::Subsonic,
        );
        server.access_token = Some("secret".into());
        server.user_id = Some("user".into());
        server.id = Some("srv".into());
        let refs: Vec<String> = (0..4).map(|n| format!("song-{n}")).collect();
        database
            .replace_favorites_clean("srv", &refs)
            .await
            .expect("synced likes");
        for item in &refs {
            database
                .set_favorite("srv", item, false)
                .await
                .expect("pending unlike");
        }
        let player =
            player::player::Player::try_with_sink(Box::new(player::engine::NullSink::new()))
                .expect("headless player starts");
        let session = SessionHandle::spawn_with_player(
            Arc::new(NoTracks),
            player,
            crate::PlaybackServices {
                sources: Arc::new(server::sources::Sources {
                    active: config::Source::Server("srv".into()),
                    server: Some(server),
                    ..Default::default()
                }),
                ..Default::default()
            },
        );
        let service = FavoritesService::new(database.clone(), session);

        let (switch, sync) = tokio::join!(
            service.sync(SyncReason::Activate, Import::IfMissing, None),
            service.sync(SyncReason::Manual, Import::IfMissing, None)
        );
        switch.expect("the watcher's sync");
        sync.expect("the sync that joined it");
        assert_eq!(service.started.load(Ordering::SeqCst), 1, "one sync ran");
        assert_eq!(unstars.load(Ordering::SeqCst), 4);
        assert!(
            database
                .dirty_unlikes("srv")
                .await
                .expect("pending")
                .is_empty()
        );
    }

    /// The favorites the playlist sync asks for first leave the server alone when a favorites sync has just run.
    #[tokio::test]
    async fn a_sync_after_a_fresh_one_asks_the_server_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let database = db::init(&dir.path().join("favorites.db"))
            .await
            .expect("db");
        let requests = Arc::new(AtomicU64::new(0));
        let mut server = config::MusicServer::new_with_service(
            "Navidrome".into(),
            subsonic(b"GET /rest/", requests.clone()).await,
            config::MusicService::Subsonic,
        );
        server.access_token = Some("secret".into());
        server.user_id = Some("user".into());
        server.id = Some("srv".into());
        let source = config::Source::Server("srv".into());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_secs();
        database
            .meta_put("fav_pull", "srv", &now.to_string())
            .await
            .expect("pull stamp");
        crate::auto_sync::mark_synced(&database, JobKind::FavoritesSync, &source).await;
        let player =
            player::player::Player::try_with_sink(Box::new(player::engine::NullSink::new()))
                .expect("headless player starts");
        let session = SessionHandle::spawn_with_player(
            Arc::new(NoTracks),
            player,
            crate::PlaybackServices {
                sources: Arc::new(server::sources::Sources {
                    active: source,
                    server: Some(server),
                    ..Default::default()
                }),
                ..Default::default()
            },
        );
        let service = FavoritesService::new(database.clone(), session);

        service
            .sync(SyncReason::Interval, Import::UnlessRecent, None)
            .await
            .expect("sync");
        assert_eq!(requests.load(Ordering::SeqCst), 0);
    }
}
