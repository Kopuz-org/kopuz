//! Scrobbling, ported from `hooks/src/scrobble_scheduler.rs`: on each track
//! commit, announce now-playing, wait for the Last.fm threshold (240 s or
//! half the track) while the same session keeps playing, then submit to the
//! native source, Last.fm, Libre.fm, and ListenBrainz, with the transient
//! failure queue and its drain-on-success behavior intact.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use api::{Intent, Phase};
use reader::Track;

use crate::session::SessionHandle;

const NOW_PLAYING_INTERVAL_SECS: u64 = 30;

pub struct Scrobbler {
    db: db::Db,
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
    pub fn new(db: db::Db) -> Arc<Self> {
        Arc::new(Self {
            db,
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

    /// Wall-clock accumulation of playing time, aborted when the session
    /// moves on, mirroring the hooks `wait_for_playtime`.
    async fn wait_for_playtime(session: &SessionHandle, threshold: Duration, token: u64) -> bool {
        let mut states = session.state_watch();
        let mut played = Duration::ZERO;
        while played < threshold {
            if !Self::token_live(session, token) {
                return false;
            }
            if states.borrow().phase != Phase::Playing {
                if states.changed().await.is_err() {
                    return false;
                }
                continue;
            }
            let started = tokio::time::Instant::now();
            tokio::select! {
                _ = tokio::time::sleep(threshold - played) => return Self::token_live(session, token),
                changed = states.changed() => {
                    if changed.is_err() {
                        return false;
                    }
                    played += started.elapsed();
                }
            }
        }
        true
    }

    /// Called by the session on every committed track.
    pub fn track_committed(self: &Arc<Self>, track: Track, token: u64) {
        let Some(session) = self.session().cloned() else {
            return;
        };
        let config = session.config_watch().borrow().clone();
        let uid = track.id.uid();
        if track.duration == u64::MAX {
            return;
        }
        let item_id = match &track.id {
            reader::TrackId::Server { item_id, .. } => Some(item_id.clone()),
            reader::TrackId::Local(_) => None,
        };
        let include_ids = item_id.is_none();

        self.spawn_playing_now_heartbeat(&session, &track, token, &config, include_ids);

        let scrobbler = self.clone();
        let span = tracing::info_span!(
            "scrobble.submit",
            track = item_id.as_deref().unwrap_or(uid.as_str())
        );
        tokio::spawn(tracing::Instrument::instrument(
            async move {
                scrobbler
                    .submit_flow(session, track, token, item_id, include_ids, config)
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
        config: config::AppConfig,
    ) {
        let duration_secs = track.duration;
        let threshold_secs = std::cmp::min(240, duration_secs / 2);
        let started_at = scrobble::musicbrainz::now_unix();

        if duration_secs < 30 {
            return;
        }
        if track.artist.trim().is_empty() || track.title.trim().is_empty() {
            return;
        }

        let source = item_id
            .as_deref()
            .map(|_| server::source::active(self.db.clone(), &config));
        if let (Some(source), Some(id)) = (&source, item_id.as_deref())
            && let Err(error) = source.scrobble_now_playing(id).await
        {
            tracing::warn!(%error, "now-playing scrobble failed");
        }

        let lastfm_api_key = config.lastfm_api_key.clone();
        let lastfm_api_secret = config.lastfm_api_secret.clone();
        let lastfm_session_key = config.lastfm_session_key.clone();
        let has_lastfm = !lastfm_api_key.is_empty() && !lastfm_api_secret.is_empty();
        if has_lastfm {
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

        let librefm_session_key = config.librefm_session_key.clone();
        let has_librefm = !librefm_session_key.is_empty();
        if has_librefm {
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

        if !Self::wait_for_playtime(&session, Duration::from_secs(threshold_secs), token).await {
            tracing::info!(
                "scrobble skipped: track changed before {threshold_secs}s: {} - {}",
                track.artist,
                track.title
            );
            return;
        }

        if let (Some(source), Some(id)) = (&source, item_id.as_deref()) {
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

        let token_mb = config.musicbrainz_token.clone();
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
                    tracing::info!("MusicBrainz scrobbled: {} - {}", track.artist, track.title);
                }
                Err(error) => {
                    tracing::warn!(%error, "MusicBrainz scrobble failed");
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
            self.drain_queue(&config).await;
        }
    }

    fn spawn_playing_now_heartbeat(
        &self,
        session: &SessionHandle,
        track: &Track,
        token: u64,
        config: &config::AppConfig,
        include_ids: bool,
    ) {
        if track.duration < 30 {
            return;
        }
        let mb_token = config.musicbrainz_token.clone();
        if mb_token.trim().is_empty() {
            return;
        }
        let session = session.clone();
        let track = track.clone();
        let span = tracing::info_span!("scrobble.playing_now", track = track.id.uid().as_str());
        tokio::spawn(tracing::Instrument::instrument(
            async move {
                let mut announced = false;
                let mut states = session.state_watch();
                'heartbeat: loop {
                    if !Self::token_live(&session, token) {
                        return;
                    }
                    if states.borrow().phase != Phase::Playing {
                        if states.changed().await.is_err() {
                            return;
                        }
                        continue;
                    }
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
                    let deadline = tokio::time::Instant::now()
                        + Duration::from_secs(NOW_PLAYING_INTERVAL_SECS);
                    loop {
                        tokio::select! {
                            _ = tokio::time::sleep_until(deadline) => break,
                            changed = states.changed() => {
                                if changed.is_err() || !Self::token_live(&session, token) {
                                    return;
                                }
                                if states.borrow().phase != Phase::Playing {
                                    continue 'heartbeat;
                                }
                            }
                        }
                    }
                }
            },
            span,
        ));
    }

    /// Startup drain: anything queued while offline gets resubmitted once.
    pub async fn drain_queue(&self, config: &config::AppConfig) {
        let has_lastfm = !config.lastfm_api_key.is_empty() && !config.lastfm_api_secret.is_empty();
        let creds = scrobble::queue::Credentials {
            lastfm: has_lastfm.then(|| {
                (
                    config.lastfm_api_key.clone(),
                    config.lastfm_api_secret.clone(),
                    config.lastfm_session_key.clone(),
                )
            }),
            librefm_session_key: (!config.librefm_session_key.is_empty())
                .then(|| config.librefm_session_key.clone()),
            listenbrainz_token: (!config.musicbrainz_token.trim().is_empty())
                .then(|| config.musicbrainz_token.clone()),
        };
        scrobble::queue::drain(&self.db, &creds).await;
    }
}
