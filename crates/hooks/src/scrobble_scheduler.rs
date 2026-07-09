use config::AppConfig;
use dioxus::core::Runtime;
use dioxus::logger::tracing::Instrument;
use dioxus::prelude::*;
use reader::Track;
use std::collections::HashMap;
use std::time::Duration;

const NOW_PLAYING_INTERVAL_SECS: u64 = 30;
const NOW_PLAYING_MAX_IDLE_SECS: u64 = 600;

#[derive(Clone, Copy)]
pub struct ScrobbleOptions {
    pub include_librefm: bool,
    pub include_musicbrainz_ids: bool,
}

impl ScrobbleOptions {
    pub const REMOTE_NATIVE: Self = Self {
        include_librefm: true,
        include_musicbrainz_ids: false,
    };

    pub const REMOTE_WEB: Self = Self {
        include_librefm: false,
        include_musicbrainz_ids: false,
    };

    pub const LOCAL: Self = Self {
        include_librefm: false,
        include_musicbrainz_ids: true,
    };
}

pub fn schedule(
    track: Track,
    item_id: Option<String>,
    config: Signal<AppConfig>,
    play_generation: Signal<usize>,
    generation: usize,
    is_playing: Signal<bool>,
    active_source: Option<Signal<::server::source::ActiveSource>>,
    options: ScrobbleOptions,
) {
    let duration_secs = track.duration;
    let threshold_secs = std::cmp::min(240, duration_secs / 2);
    let started_at = scrobble::musicbrainz::now_unix();
    let span = tracing::info_span!(
        "scrobble.submit",
        track = item_id.as_deref().unwrap_or(track.id.uid().as_str())
    );

    schedule_playing_now_heartbeat(
        &track,
        config,
        play_generation,
        generation,
        is_playing,
        options,
    );

    spawn_in_scope(
        play_generation.origin_scope(),
        async move {
            if duration_secs < 30 {
                tracing::info!(
                    "scrobble skipped: track too short ({duration_secs}s < 30s): {} - {}",
                    track.artist,
                    track.title
                );
                return;
            }

            if track.artist.trim().is_empty() || track.title.trim().is_empty() {
                tracing::info!(
                    "scrobble skipped: missing artist or title metadata: {:?} - {:?}",
                    track.artist,
                    track.title
                );
                return;
            }

            if let (Some(source), Some(id)) = (active_source, item_id.as_deref()) {
                let source = source.peek().clone();
                if let Err(error) = source.scrobble_now_playing(id).await {
                    tracing::warn!("now-playing scrobble failed: {}", error);
                }
            }

            let lastfm_api_key = config.read().lastfm_api_key.clone();
            let lastfm_api_secret = config.read().lastfm_api_secret.clone();
            let lastfm_session_key = config.read().lastfm_session_key.clone();
            let has_lastfm = !lastfm_api_key.is_empty() && !lastfm_api_secret.is_empty();

            if has_lastfm {
                let playing_now = scrobble::lastfm::make_playing_now(
                    &track.artist,
                    &track.title,
                    Some(&track.album),
                );
                if let Err(error) = scrobble::lastfm::submit_now_playing(
                    &lastfm_api_key,
                    &lastfm_api_secret,
                    &lastfm_session_key,
                    &playing_now,
                )
                .await
                {
                    tracing::warn!("Last.fm now playing failed: {}", error);
                }
            }

            let librefm_session_key = config.read().librefm_session_key.clone();
            let has_librefm = options.include_librefm && !librefm_session_key.is_empty();

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
                    tracing::warn!("Libre.fm now playing failed: {}", error);
                }
            }

            let reached = wait_for_playtime(
                Duration::from_secs(threshold_secs),
                play_generation,
                generation,
                is_playing,
            )
            .await;

            if !reached {
                tracing::info!(
                    "scrobble skipped: track changed before {threshold_secs}s of playback: {} - {}",
                    track.artist,
                    track.title
                );
                return;
            }

            if let (Some(source), Some(id)) = (active_source, item_id.as_deref()) {
                let source = source.peek().clone();
                match source.scrobble(id).await {
                    Ok(_) => tracing::info!("scrobbled: {} - {}", track.artist, track.title),
                    Err(error) => tracing::warn!("scrobble failed: {}", error),
                }
            }

            // Offline queue bookkeeping (issue #335): scrobbles that fail with
            // a transient error are queued with the original listen timestamp
            // (`started_at`) and resubmitted later; one success means we're
            // online, so drain the backlog.
            let queue_path = scrobble::queue::default_queue_path();
            let mut scrobble_ok = false;

            if has_lastfm {
                let scrobble = scrobble::lastfm::make_scrobble_at(
                    &track.artist,
                    &track.title,
                    Some(&track.album),
                    started_at,
                );
                match scrobble::lastfm::submit_scrobble(
                    &lastfm_api_key,
                    &lastfm_api_secret,
                    &lastfm_session_key,
                    &scrobble,
                )
                .await
                {
                    Ok(_) => {
                        scrobble_ok = true;
                        tracing::info!("Last.fm scrobbled: {} - {}", track.artist, track.title)
                    }
                    Err(error) => {
                        tracing::warn!("Last.fm scrobble failed: {}", error);
                        if scrobble::queue::is_transient(&error)
                            && let Some(qp) = &queue_path
                        {
                            scrobble::queue::enqueue(
                                qp,
                                scrobble::queue::Service::LastFm,
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
                let scrobble = scrobble::librefm::make_scrobble_at(
                    &track.artist,
                    &track.title,
                    Some(&track.album),
                    started_at,
                );
                match scrobble::librefm::submit_scrobble(
                    scrobble::librefm::API_KEY,
                    scrobble::librefm::API_SECRET,
                    &librefm_session_key,
                    &scrobble,
                )
                .await
                {
                    Ok(_) => {
                        scrobble_ok = true;
                        tracing::info!("Libre.fm scrobbled: {} - {}", track.artist, track.title)
                    }
                    Err(error) => {
                        tracing::warn!("Libre.fm scrobble failed: {}", error);
                        if scrobble::queue::is_transient(&error)
                            && let Some(qp) = &queue_path
                        {
                            scrobble::queue::enqueue(
                                qp,
                                scrobble::queue::Service::LibreFm,
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

            let token = config.read().musicbrainz_token.clone();
            if !token.trim().is_empty() {
                let info = listen_additional_info(&track, options.include_musicbrainz_ids);
                let queued_info: serde_json::Map<String, serde_json::Value> = info
                    .iter()
                    .map(|(k, v)| ((*k).to_string(), v.clone()))
                    .collect();
                let listen = scrobble::musicbrainz::make_listen(
                    &track.artist,
                    &track.title,
                    Some(&track.album),
                    Some(info),
                    started_at,
                );
                match scrobble::musicbrainz::submit_listens(&token, vec![listen], "single").await {
                    Ok(_) => {
                        scrobble_ok = true;
                        tracing::info!("MusicBrainz scrobbled: {} - {}", track.artist, track.title)
                    }
                    Err(error) => {
                        tracing::warn!("MusicBrainz scrobble failed: {}", error);
                        if scrobble::queue::is_transient(&error)
                            && let Some(qp) = &queue_path
                        {
                            scrobble::queue::enqueue(
                                qp,
                                scrobble::queue::Service::ListenBrainz,
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

            // One success means we're online again: flush queued scrobbles.
            if scrobble_ok && let Some(qp) = &queue_path {
                let musicbrainz_token = config.read().musicbrainz_token.clone();
                let creds = scrobble::queue::Credentials {
                    lastfm: has_lastfm.then(|| {
                        (
                            lastfm_api_key.clone(),
                            lastfm_api_secret.clone(),
                            lastfm_session_key.clone(),
                        )
                    }),
                    librefm_session_key: has_librefm.then(|| librefm_session_key.clone()),
                    listenbrainz_token: (!musicbrainz_token.trim().is_empty())
                        .then(|| musicbrainz_token.clone()),
                };
                scrobble::queue::drain(qp, &creds).await;
            }
        }
        .instrument(span),
    );
}

fn schedule_playing_now_heartbeat(
    track: &Track,
    config: Signal<AppConfig>,
    play_generation: Signal<usize>,
    generation: usize,
    is_playing: Signal<bool>,
    options: ScrobbleOptions,
) {
    if track.duration < 30 {
        return;
    }

    let token = config.read().musicbrainz_token.clone();
    if token.trim().is_empty() {
        return;
    }

    let track = track.clone();
    let include_ids = options.include_musicbrainz_ids;
    let span = tracing::info_span!("scrobble.playing_now", track = track.id.uid().as_str());

    spawn_in_scope(
        play_generation.origin_scope(),
        async move {
            let mut announced = false;
            let mut idle_secs: u64 = 0;
            loop {
                if *play_generation.read() != generation {
                    return;
                }

                if *is_playing.read() {
                    idle_secs = 0;
                    let now_info = listen_additional_info(&track, include_ids);
                    let playing_now = scrobble::musicbrainz::make_playing_now(
                        &track.artist,
                        &track.title,
                        Some(&track.album),
                        Some(now_info),
                    );
                    let sent = scrobble::musicbrainz::submit_listens(
                        &token,
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
        }
        .instrument(span),
    );
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

fn spawn_in_scope(scope: ScopeId, fut: impl std::future::Future<Output = ()> + 'static) {
    Runtime::current().in_scope(scope, || {
        spawn(fut);
    });
}

async fn wait_for_playtime(
    threshold: Duration,
    play_generation: Signal<usize>,
    generation: usize,
    is_playing: Signal<bool>,
) -> bool {
    let tick = Duration::from_secs(1);
    let mut played = Duration::ZERO;

    while played < threshold {
        tokio::time::sleep(tick).await;

        if *play_generation.read() != generation {
            return false;
        }

        if *is_playing.read() {
            played += tick;
        }
    }

    true
}
