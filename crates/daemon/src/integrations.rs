//! Playback integrations ported from the event pump: the Jellyfin session
//! reporter and the Discord presence projector, plus the source-backed
//! [`PlaybackRecorder`].
//!
//! Both tasks are event-driven off the session's state stream with their
//! timers gated on activity, so an idle daemon takes no wakeups from them.

use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use api::{ApiEvent, Phase, PlayerState};
use reader::Track;
use tokio::sync::{broadcast, watch};

use crate::ConfigService;
use crate::session::{PlaybackRecorder, SessionHandle};

const DISCORD_APP_ID: &str = "1470087339639443658";
const JELLYFIN_REPORT_SECS: u64 = 5;
const JELLYFIN_KEEPALIVE_TICKS: u32 = 6;
const DISCORD_TICK_SECS: u64 = 30;

#[derive(Clone, PartialEq, Eq)]
enum CredentialIdentity {
    YtMusic(String),
    Spotify(String),
}

fn credential_identity(config: &config::AppConfig) -> Option<CredentialIdentity> {
    let server = config.server.as_ref()?;
    let token = server
        .access_token
        .as_deref()
        .filter(|token| !token.is_empty())?;
    match server.service {
        config::MusicService::YtMusic => {
            server::ytmusic::derive_user_id(token).map(CredentialIdentity::YtMusic)
        }
        config::MusicService::Spotify => Some(CredentialIdentity::Spotify(
            server.id.clone().unwrap_or_else(|| server.url.clone()),
        )),
        _ => None,
    }
}

async fn maintain_credentials(
    config_service: &Arc<ConfigService>,
    session: &SessionHandle,
    identity: &CredentialIdentity,
) {
    let snapshot = config_service.snapshot().await;
    let Some(server) = snapshot.server.as_ref() else {
        return;
    };
    let Some(previous) = server.access_token.clone() else {
        return;
    };
    let Some(server_id) = server.id.clone() else {
        tracing::warn!("active source credential cannot be rotated without a server id");
        return;
    };
    let rotated = match identity {
        CredentialIdentity::YtMusic(_) => {
            match server::ytmusic::verify_session_keepalive::tick(&previous).await {
                Ok(Some(rotated)) => Some(rotated),
                Ok(None) => None,
                Err(error) => {
                    tracing::warn!(%error, "YT Music session keepalive failed");
                    None
                }
            }
        }
        CredentialIdentity::Spotify(_) => {
            match server::spotify::auth::refresh_packed(&previous, server.url.clone()).await {
                Ok(rotated) => Some(rotated),
                Err(error) => {
                    tracing::warn!(%error, "Spotify credential refresh failed");
                    None
                }
            }
        }
    };
    let Some(rotated) = rotated else {
        return;
    };
    let updated = match config_service
        .rotate_active_server_credential(&server_id, &previous, rotated)
        .await
    {
        Ok(Some(updated)) => updated,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(%error, "rotated source credential could not be persisted");
            return;
        }
    };
    session.set_active_source(Some(config_service.playback_source(&updated)));
    session.set_config(updated, vec!["servers".to_string()]);
}

/// Headless source credential maintenance. The task parks on the config watch
/// when no supported authenticated source is active, and only arms a timer for
/// the active credential.
pub fn spawn_credential_maintenance(
    config_service: Arc<ConfigService>,
    session: SessionHandle,
) -> tokio::task::JoinHandle<()> {
    let mut updates = config_service.subscribe();
    tokio::spawn(async move {
        loop {
            let Some(identity) = credential_identity(&updates.borrow().clone()) else {
                if updates.changed().await.is_err() {
                    return;
                }
                continue;
            };
            maintain_credentials(&config_service, &session, &identity).await;
            let interval = match identity {
                CredentialIdentity::YtMusic(_) => Duration::from_secs(300),
                CredentialIdentity::Spotify(_) => Duration::from_secs(1800),
            };
            let deadline = tokio::time::Instant::now() + interval;
            loop {
                tokio::select! {
                    _ = tokio::time::sleep_until(deadline) => break,
                    changed = updates.changed() => {
                        if changed.is_err() {
                            return;
                        }
                        if credential_identity(&updates.borrow().clone()).as_ref() != Some(&identity) {
                            break;
                        }
                    }
                }
            }
        }
    })
}

/// Durable recents + listen counts over the active source, matching the
/// pump: recents record under the DB key, listen counts under the uid.
pub struct SourceRecorder {
    db: db::Db,
    session: OnceLock<SessionHandle>,
}

impl SourceRecorder {
    pub fn new(db: db::Db) -> Self {
        Self {
            db,
            session: OnceLock::new(),
        }
    }

    pub fn attach_session(&self, session: SessionHandle) {
        let _ = self.session.set(session);
    }

    fn active_source(&self) -> Option<server::source::ActiveSource> {
        let config = self.session.get()?.config_watch().borrow().clone();
        Some(server::source::ActiveSource::from(server::source::active(
            self.db.clone(),
            &config,
        )))
    }
}

#[async_trait::async_trait]
impl PlaybackRecorder for SourceRecorder {
    async fn record_recent(&self, track: &Track) {
        let Some(source) = self.active_source() else {
            return;
        };
        if let Err(error) = source.record_recent(&track.id.key()).await {
            tracing::warn!(%error, "recent record failed");
        }
    }

    async fn bump_listen_count(&self, track: &Track) {
        let Some(source) = self.active_source() else {
            return;
        };
        if let Err(error) = source.bump_listen_count(&track.id.uid()).await {
            tracing::warn!(%error, "listen count persist failed");
        }
    }
}

fn interpolated_ms(state: &PlayerState, received: Instant) -> u64 {
    let Some(anchor) = state.position else {
        return 0;
    };
    if !anchor.playing {
        return anchor.ms;
    }
    anchor.ms + received.elapsed().as_millis() as u64
}

async fn next_state(
    events: &mut broadcast::Receiver<(u64, ApiEvent)>,
) -> Option<Option<Box<PlayerState>>> {
    match events.recv().await {
        Ok((_, ApiEvent::PlayerState(state))) => Some(Some(state)),
        Ok(_) => Some(None),
        Err(broadcast::error::RecvError::Lagged(_)) => Some(None),
        Err(broadcast::error::RecvError::Closed) => None,
    }
}

/// Jellyfin session reporting: keepalive while a Jellyfin track is loaded,
/// start/stop on track change, progress every 5 s and on play/pause flips.
/// Deviation from the pump, per the resource budget: the 30 s keepalive only
/// runs while a Jellyfin session exists instead of forever.
pub fn spawn_jellyfin_reporter(
    session: &SessionHandle,
    db: db::Db,
    config: watch::Receiver<config::AppConfig>,
) -> tokio::task::JoinHandle<()> {
    let mut events = session.subscribe();
    tokio::spawn(async move {
        let mut active: Option<(config::Source, String, server::source::ActiveSource)> = None;
        let mut last_state: Option<(Box<PlayerState>, Instant)> = None;
        let mut was_playing = false;
        let mut ticks_until_keepalive = 0u32;
        let mut report = tokio::time::interval(Duration::from_secs(JELLYFIN_REPORT_SECS));
        report.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            let current_config = config.borrow().clone();
            let jellyfin_configured = current_config
                .server
                .as_ref()
                .is_some_and(|server| server.service == config::MusicService::Jellyfin);

            tokio::select! {
                state = next_state(&mut events) => {
                    let Some(state) = state else { break };
                    let Some(state) = state else { continue };
                    let received = Instant::now();
                    let current_id = jellyfin_configured
                        .then(|| state.track.as_ref().map(|track| track.key.clone()))
                        .flatten();
                    let current_identity = current_id
                        .as_ref()
                        .map(|id| (current_config.active_source.clone(), id.clone()));
                    let playing = state.phase == Phase::Playing;
                    let position_ms = interpolated_ms(&state, received);

                    if current_identity
                        != active
                            .as_ref()
                            .map(|(source, id, _)| (source.clone(), id.clone()))
                    {
                        if let Some((_, old, source)) = active.take() {
                            let ticks = last_state
                                .as_ref()
                                .map(|(state, received)| interpolated_ms(state, *received))
                                .unwrap_or_default()
                                * 10_000;
                            tokio::spawn(async move {
                                let _ = source.report_playback_stopped(&old, ticks).await;
                            });
                        }
                        if let Some(new) = current_id {
                            let source = server::source::ActiveSource::from(
                                server::source::active(db.clone(), &current_config),
                            );
                            let start_source = source.clone();
                            let source_key = current_config.active_source.clone();
                            active = Some((source_key, new.clone(), source));
                            tokio::spawn(async move {
                                let _ = start_source.report_playback_start(&new).await;
                            });
                        }
                        ticks_until_keepalive = 0;
                    } else if playing != was_playing
                        && let Some((_, id, source)) = active.as_ref()
                    {
                        let source = source.clone();
                        let id = id.clone();
                        let ticks = position_ms * 10_000;
                        tokio::spawn(async move {
                            let _ = source
                                .report_playback_progress(&id, ticks, !playing)
                                .await;
                        });
                    }
                    was_playing = playing;
                    last_state = Some((state, received));
                }
                _ = report.tick(), if jellyfin_configured && active.is_some() => {
                    let Some((_, id, source)) = active.as_ref() else { continue };
                    let id = id.clone();
                    let source = source.clone();
                    let position_ms = last_state
                        .as_ref()
                        .map(|(state, received)| interpolated_ms(state, *received))
                        .unwrap_or_default();
                    let paused = !was_playing;
                    let progress_source = source.clone();
                    tokio::spawn(async move {
                        let _ = progress_source
                            .report_playback_progress(&id, position_ms * 10_000, paused)
                            .await;
                    });
                    if ticks_until_keepalive == 0 {
                        ticks_until_keepalive = JELLYFIN_KEEPALIVE_TICKS;
                        let source = source.clone();
                        tokio::spawn(async move {
                            let _ = source.keepalive().await;
                        });
                    }
                    ticks_until_keepalive -= 1;
                }
            }
        }
    })
}

struct DiscordState {
    last_title: String,
    was_playing: bool,
    last_enabled: bool,
    last_source: Option<String>,
    cover: Option<String>,
    cover_sent: bool,
    cover_song_key: String,
    cover_lookup_attempted: bool,
    cover_task: Option<tokio::task::JoinHandle<()>>,
}

impl DiscordState {
    fn cancel_cover_lookup(&mut self) {
        if let Some(task) = self.cover_task.take() {
            task.abort();
        }
    }
}

impl Drop for DiscordState {
    fn drop(&mut self) {
        self.cancel_cover_lookup();
    }
}

/// Discord presence projection, ported from the pump: now-playing on play,
/// paused card when enabled, cleared when disabled, with async cover-art
/// resolution keyed by song so a late result never stamps the wrong track.
pub fn spawn_discord_presence(
    session: &SessionHandle,
    config: watch::Receiver<config::AppConfig>,
) -> tokio::task::JoinHandle<()> {
    let mut events = session.subscribe();
    tokio::spawn(async move {
        let presence = match discord_presence::Presence::new(DISCORD_APP_ID) {
            Ok(presence) => Arc::new(presence),
            Err(error) => {
                tracing::info!(%error, "discord presence unavailable");
                return;
            }
        };
        let (cover_tx, mut cover_rx) =
            tokio::sync::mpsc::unbounded_channel::<(String, Option<String>)>();
        let mut discord = DiscordState {
            last_title: String::new(),
            was_playing: false,
            last_enabled: false,
            last_source: None,
            cover: None,
            cover_sent: false,
            cover_song_key: String::new(),
            cover_lookup_attempted: false,
            cover_task: None,
        };
        let mut last_state: Option<(Box<PlayerState>, Instant)> = None;
        let mut keepalive = tokio::time::interval(Duration::from_secs(DISCORD_TICK_SECS));
        keepalive.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                state = next_state(&mut events) => {
                    let Some(state) = state else { break };
                    let Some(state) = state else { continue };
                    let received = Instant::now();
                    project(&presence, &config.borrow(), &state, received, &mut discord, &cover_tx);
                    last_state = Some((state, received));
                }
                resolved = cover_rx.recv() => {
                    let Some((song_key, url)) = resolved else { break };
                    if song_key == discord.cover_song_key {
                        discord.cover_task.take();
                        discord.cover = url;
                        discord.cover_sent = false;
                        if let Some((state, received)) = &last_state {
                            project(&presence, &config.borrow(), state, *received, &mut discord, &cover_tx);
                        }
                    }
                }
                _ = keepalive.tick(), if discord.last_enabled => {
                    presence.tick();
                }
            }
        }
    })
}

fn project(
    presence: &discord_presence::Presence,
    config: &config::AppConfig,
    state: &PlayerState,
    received: Instant,
    discord: &mut DiscordState,
    cover_tx: &tokio::sync::mpsc::UnboundedSender<(String, Option<String>)>,
) {
    let enabled = config.discord_presence.unwrap_or(true);
    let paused_enabled = config.discord_presence_paused.unwrap_or(true);
    let source_name = config.discord_presence_source.unwrap_or(true).then(|| {
        config
            .active_service()
            .map_or("Local", |service| service.display_name())
            .to_string()
    });
    let source_changed = discord.last_source != source_name;
    let playing = state.phase == Phase::Playing;
    let track = state.track.as_ref();
    let title = track.map(|t| t.title.clone()).unwrap_or_default();
    let artist = track.map(|t| t.artist.clone()).unwrap_or_default();
    let album = track.map(|t| t.album.clone()).unwrap_or_default();
    let song_key = format!("{title}|{artist}|{album}");
    if song_key != discord.cover_song_key {
        discord.cancel_cover_lookup();
        discord.cover_song_key = song_key.clone();
        discord.cover_lookup_attempted = false;
        discord.cover = None;
        discord.cover_sent = false;
    }
    if !enabled {
        discord.cancel_cover_lookup();
        discord.cover_lookup_attempted = false;
    }
    let duration_secs = track
        .and_then(|t| t.duration_ms)
        .map(|ms| ms / 1000)
        .unwrap_or(u64::MAX);
    let progress_secs = if duration_secs == u64::MAX {
        0
    } else {
        interpolated_ms(state, received) / 1000
    };

    if playing {
        if enabled && !title.is_empty() && !discord.cover_lookup_attempted {
            discord.cover_lookup_attempted = true;
            let tx = cover_tx.clone();
            let (artist_c, album_c) = (artist.clone(), album.clone());
            discord.cover_task = Some(tokio::spawn(async move {
                let resolved = discord_presence::cover_art::resolve_cover_art_url_cached(
                    None, &artist_c, &album_c,
                )
                .await;
                let _ = tx.send((song_key, resolved));
            }));
        }
        if enabled {
            let song_changed = title != discord.last_title;
            let resumed = !discord.was_playing;
            let toggled_on = !discord.last_enabled;
            let cover_just_resolved = discord.cover.is_some() && !discord.cover_sent;
            if song_changed || resumed || toggled_on || cover_just_resolved || source_changed {
                discord.last_title = title.clone();
                discord.last_source = source_name.clone();
                let _ = presence.set_now_playing(
                    &title,
                    &artist,
                    &album,
                    progress_secs,
                    duration_secs,
                    discord.cover.as_deref(),
                    source_name.as_deref(),
                );
                if discord.cover.is_some() {
                    discord.cover_sent = true;
                }
            }
        } else if discord.last_enabled {
            let _ = presence.clear_activity();
        }
    } else if discord.was_playing {
        if enabled && paused_enabled {
            discord.last_source = source_name.clone();
            let _ = presence.set_paused(
                &title,
                &artist,
                &album,
                discord.cover.as_deref(),
                source_name.as_deref(),
            );
        } else if discord.last_enabled || !paused_enabled {
            let _ = presence.clear_activity();
        }
    } else if !enabled && discord.last_enabled {
        let _ = presence.clear_activity();
    } else if enabled
        && (!discord.last_enabled || (source_changed && paused_enabled))
        && !title.is_empty()
    {
        discord.last_source = source_name.clone();
        let _ = presence.set_paused(
            &title,
            &artist,
            &album,
            discord.cover.as_deref(),
            source_name.as_deref(),
        );
    }

    discord.was_playing = playing;
    discord.last_enabled = enabled;
}
