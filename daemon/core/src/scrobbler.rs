//! Announce now-playing on track commit and scrobble after 240 seconds or half
//! the track's duration, provided the same session keeps playing. Transient
//! failures are queued and retried after a successful submission.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use api::{Intent, Phase};
use reader::Track;
use server::playback_ref::PlaybackItemRef;

use crate::credentials::{CredentialStore, Credentials};
use crate::session::SessionHandle;

const NOW_PLAYING_INTERVAL_SECS: u64 = 30;
const NOW_PLAYING_MAX_IDLE_SECS: u64 = 600;

/// How long a track has to actually play before it counts as listened: four
/// minutes or half the track, whichever comes first. Scrobbles and the
/// library's own play counts both go by this, so the two agree. A track that
/// reports no duration has to play 30 seconds rather than counting at once.
pub(crate) fn listen_threshold(duration_secs: u64) -> Duration {
    if duration_secs == 0 {
        return Duration::from_secs(30);
    }
    Duration::from_secs(duration_secs.div_ceil(2).min(240))
}

/// Accumulate playing time until `threshold`, or give up once the track is no
/// longer the one being heard. Paused stretches do not count.
///
/// A gapless preload or a crossfade publishes the next track's token while
/// this one is still playing, so the track also stays live as long as the
/// queue sits at the position it was heard at with the same track there.
pub(crate) async fn accumulate_playtime(
    state: impl Fn() -> api::PlayerState,
    threshold: Duration,
    token: u64,
    uid: &str,
) -> bool {
    let tick = Duration::from_secs(1);
    let mut played = Duration::ZERO;
    let mut heard_at = None;
    while played < threshold {
        tokio::time::sleep(tick).await;
        let state = state();
        let current = matches!(
            state.intent,
            Intent::Committed { token: current } | Intent::Loading { token: current, .. }
                if current == token
        );
        let same_track = state.track.as_ref().is_some_and(|track| track.uid == uid);
        // A preload can land before the first check, so the first sighting of the
        // track itself also marks where it is heard.
        if current || (heard_at.is_none() && same_track) {
            heard_at = state.queue.index;
        }
        let still_heard = heard_at.is_some() && state.queue.index == heard_at && same_track;
        if !current && !still_heard {
            return false;
        }
        if state.phase == Phase::Playing {
            played += tick;
        }
    }
    true
}

pub struct Scrobbler {
    db: db::Db,
    credentials: CredentialStore,
    session: OnceLock<SessionHandle>,
}

fn listen_additional_info(
    track: &Track,
    include_ids: bool,
) -> HashMap<&'static str, serde_json::Value> {
    let mut map = HashMap::new();
    map.insert("media_player", serde_json::Value::from("kopuz"));
    map.insert("submission_client", serde_json::Value::from("kopuz"));
    map.insert(
        "submission_client_version",
        serde_json::Value::from(env!("CARGO_PKG_VERSION")),
    );
    if track.duration > 0 {
        map.insert(
            "duration_ms",
            serde_json::Value::from(track.duration * 1000),
        );
    }
    if include_ids {
        if let Some(mbid) = &track.musicbrainz_release_id {
            map.insert("release_mbid", serde_json::Value::from(mbid.as_str()));
        }
        if let Some(mbid) = &track.musicbrainz_recording_id {
            map.insert("recording_mbid", serde_json::Value::from(mbid.as_str()));
        }
        if let Some(mbid) = &track.musicbrainz_track_id {
            map.insert("track_mbid", serde_json::Value::from(mbid.as_str()));
        }
    }
    map
}

impl Scrobbler {
    pub fn new(db: db::Db, credentials: CredentialStore) -> Arc<Self> {
        Arc::new(Self {
            db,
            credentials,
            session: OnceLock::new(),
        })
    }

    pub fn attach_session(&self, session: SessionHandle) {
        let _ = self.session.set(session);
    }

    fn session(&self) -> Option<&SessionHandle> {
        self.session.get()
    }

    /// True while the given session token is still the committed intent.
    fn token_live(session: &SessionHandle, token: u64) -> bool {
        matches!(
            session.state().intent,
            Intent::Committed { token: current } | Intent::Loading { token: current, .. }
                if current == token
        )
    }

    fn playing(session: &SessionHandle) -> bool {
        session.state().phase == Phase::Playing
    }

    async fn wait_for_playtime(
        session: &SessionHandle,
        threshold: Duration,
        token: u64,
        uid: &str,
    ) -> bool {
        accumulate_playtime(|| session.state(), threshold, token, uid).await
    }

    /// Called by the session on every committed track.
    pub fn track_committed(self: &Arc<Self>, track: Track, token: u64) {
        let Some(session) = self.session().cloned() else {
            return;
        };
        let credentials = self.credentials.snapshot();
        let uid = track.id.uid();
        let item_ref = PlaybackItemRef::from_id(&track.id);
        if item_ref.is_radio() {
            return;
        }
        let is_server = item_ref.is_server();
        let item_id = is_server.then(|| item_ref.primary_id().unwrap_or_default().to_string());
        let source = is_server.then(|| server::source::active(self.db.clone(), &session.sources()));
        let include_ids = !is_server;

        if !session.offline().offline_mode() {
            self.spawn_playing_now_heartbeat(&session, &track, token, &credentials, include_ids);
        }

        let scrobbler = self.clone();
        let span = tracing::info_span!(
            "scrobble.submit",
            track = item_id.as_deref().unwrap_or(uid.as_str())
        );
        tokio::spawn(tracing::Instrument::instrument(
            async move {
                scrobbler
                    .submit_flow(
                        session,
                        track,
                        token,
                        item_id,
                        include_ids,
                        credentials,
                        source,
                    )
                    .await;
            },
            span,
        ));
    }

    async fn submit_flow(
        &self,
        session: SessionHandle,
        track: Track,
        token: u64,
        item_id: Option<String>,
        include_ids: bool,
        credentials: Credentials,
        source: Option<Box<dyn server::source::MediaSource>>,
    ) {
        let duration_secs = track.duration;
        let threshold = listen_threshold(duration_secs);
        let started_at = scrobble::musicbrainz::now_unix();

        if duration_secs < 30 {
            return;
        }
        if track.artist.trim().is_empty() || track.title.trim().is_empty() {
            return;
        }
        // A play in offline mode is still queued below: its submissions fail as transient.
        let offline = session.offline().offline_mode();

        if !offline
            && let (Some(source), Some(id)) = (&source, item_id.as_deref())
            && let Err(error) = source.scrobble_now_playing(id).await
        {
            tracing::warn!(%error, "now-playing scrobble failed");
        }

        let Credentials {
            musicbrainz_token: token_mb,
            lastfm_api_key,
            lastfm_api_secret,
            lastfm_session_key,
            librefm_session_key,
        } = credentials;
        let has_lastfm = !lastfm_api_key.is_empty() && !lastfm_api_secret.is_empty();
        if has_lastfm && !offline {
            let playing_now =
                scrobble::lastfm::make_playing_now(&track.artist, &track.title, Some(&track.album));
            if let Err(error) = scrobble::lastfm::submit_now_playing(
                &lastfm_api_key,
                &lastfm_api_secret,
                &lastfm_session_key,
                &playing_now,
            )
            .await
            {
                tracing::warn!(%error, "Last.fm now playing failed");
            }
        }

        let has_librefm = !librefm_session_key.is_empty();
        if has_librefm && !offline {
            let playing_now = scrobble::librefm::make_playing_now(
                &track.artist,
                &track.title,
                Some(&track.album),
            );
            if let Err(error) = scrobble::librefm::submit_now_playing(
                scrobble::librefm::API_KEY,
                scrobble::librefm::API_SECRET,
                &librefm_session_key,
                &playing_now,
            )
            .await
            {
                tracing::warn!(%error, "Libre.fm now playing failed");
            }
        }

        if !Self::wait_for_playtime(&session, threshold, token, &track.id.uid()).await {
            tracing::info!(
                "scrobble skipped: track changed before {}s: {} - {}",
                threshold.as_secs(),
                track.artist,
                track.title
            );
            return;
        }

        if !offline && let (Some(source), Some(id)) = (&source, item_id.as_deref()) {
            match source.scrobble(id).await {
                Ok(_) => tracing::info!("scrobbled: {} - {}", track.artist, track.title),
                Err(error) => tracing::warn!(%error, "scrobble failed"),
            }
        }

        let mut scrobble_ok = false;
        if has_lastfm {
            let submission = scrobble::lastfm::make_scrobble_at(
                &track.artist,
                &track.title,
                Some(&track.album),
                started_at,
            );
            match scrobble::lastfm::submit_scrobble(
                &lastfm_api_key,
                &lastfm_api_secret,
                &lastfm_session_key,
                &submission,
            )
            .await
            {
                Ok(_) => {
                    scrobble_ok = true;
                    tracing::info!("Last.fm scrobbled: {} - {}", track.artist, track.title);
                }
                Err(error) => {
                    tracing::warn!(%error, "Last.fm scrobble failed");
                    if scrobble::queue::is_transient(&error) {
                        scrobble::queue::enqueue(
                            &self.db,
                            scrobble::queue::ScrobbleService::LastFm,
                            &track.artist,
                            &track.title,
                            Some(&track.album),
                            started_at,
                            None,
                        )
                        .await;
                    }
                }
            }
        }

        if has_librefm {
            let submission = scrobble::librefm::make_scrobble_at(
                &track.artist,
                &track.title,
                Some(&track.album),
                started_at,
            );
            match scrobble::librefm::submit_scrobble(
                scrobble::librefm::API_KEY,
                scrobble::librefm::API_SECRET,
                &librefm_session_key,
                &submission,
            )
            .await
            {
                Ok(_) => {
                    scrobble_ok = true;
                    tracing::info!("Libre.fm scrobbled: {} - {}", track.artist, track.title);
                }
                Err(error) => {
                    tracing::warn!(%error, "Libre.fm scrobble failed");
                    if scrobble::queue::is_transient(&error) {
                        scrobble::queue::enqueue(
                            &self.db,
                            scrobble::queue::ScrobbleService::LibreFm,
                            &track.artist,
                            &track.title,
                            Some(&track.album),
                            started_at,
                            None,
                        )
                        .await;
                    }
                }
            }
        }

        if !token_mb.trim().is_empty() {
            let info = listen_additional_info(&track, include_ids);
            let queued_info: serde_json::Map<String, serde_json::Value> = info
                .iter()
                .map(|(key, value)| ((*key).to_string(), value.clone()))
                .collect();
            let listen = scrobble::musicbrainz::make_listen(
                &track.artist,
                &track.title,
                Some(&track.album),
                Some(info),
                started_at,
            );
            match scrobble::musicbrainz::submit_listens(&token_mb, vec![listen], "single").await {
                Ok(_) => {
                    scrobble_ok = true;
                    tracing::info!("ListenBrainz scrobbled: {} - {}", track.artist, track.title);
                }
                Err(error) => {
                    tracing::warn!(%error, "ListenBrainz scrobble failed");
                    if scrobble::queue::is_transient(&error) {
                        scrobble::queue::enqueue(
                            &self.db,
                            scrobble::queue::ScrobbleService::ListenBrainz,
                            &track.artist,
                            &track.title,
                            Some(&track.album),
                            started_at,
                            Some(queued_info),
                        )
                        .await;
                    }
                }
            }
        }

        if scrobble_ok {
            self.drain_queue().await;
        }
    }

    fn spawn_playing_now_heartbeat(
        &self,
        session: &SessionHandle,
        track: &Track,
        token: u64,
        credentials: &Credentials,
        include_ids: bool,
    ) {
        if track.duration < 30 {
            return;
        }
        let mb_token = credentials.musicbrainz_token.clone();
        if mb_token.trim().is_empty() {
            return;
        }
        let session = session.clone();
        let track = track.clone();
        let span = tracing::info_span!("scrobble.playing_now", track = track.id.uid().as_str());
        tokio::spawn(tracing::Instrument::instrument(
            async move {
                let mut announced = false;
                let mut idle_secs: u64 = 0;
                loop {
                    if !Self::token_live(&session, token) {
                        return;
                    }
                    if Self::playing(&session) {
                        idle_secs = 0;
                        let now_info = listen_additional_info(&track, include_ids);
                        let playing_now = scrobble::musicbrainz::make_playing_now(
                            &track.artist,
                            &track.title,
                            Some(&track.album),
                            Some(now_info),
                        );
                        let sent = scrobble::musicbrainz::submit_listens(
                            &mb_token,
                            vec![playing_now],
                            "playing_now",
                        )
                        .await
                        .is_ok();
                        if sent && !announced {
                            announced = true;
                            tracing::info!(
                                "ListenBrainz playing now: {} - {}",
                                track.artist,
                                track.title
                            );
                        }
                    } else {
                        idle_secs += NOW_PLAYING_INTERVAL_SECS;
                        if idle_secs >= NOW_PLAYING_MAX_IDLE_SECS {
                            return;
                        }
                    }
                    tokio::time::sleep(Duration::from_secs(NOW_PLAYING_INTERVAL_SECS)).await;
                }
            },
            span,
        ));
    }

    /// Startup drain: anything queued while offline gets resubmitted once.
    pub async fn drain_queue(&self) {
        if self
            .session()
            .is_some_and(|session| session.offline().offline_mode())
        {
            return;
        }
        let held = self.credentials.snapshot();
        let has_lastfm = !held.lastfm_api_key.is_empty() && !held.lastfm_api_secret.is_empty();
        let creds = scrobble::queue::Credentials {
            lastfm: has_lastfm.then(|| {
                (
                    held.lastfm_api_key.clone(),
                    held.lastfm_api_secret.clone(),
                    held.lastfm_session_key.clone(),
                )
            }),
            librefm_session_key: (!held.librefm_session_key.is_empty())
                .then(|| held.librefm_session_key.clone()),
            listenbrainz_token: (!held.musicbrainz_token.trim().is_empty())
                .then(|| held.musicbrainz_token.clone()),
        };
        scrobble::queue::drain(&self.db, &creds).await;
    }
}
