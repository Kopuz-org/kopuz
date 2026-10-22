//! Playback integrations ported from the event pump: the Jellyfin session
//! reporter and the Discord presence projector, plus the source-backed
//! [`PlaybackRecorder`].
//!
//! Both tasks are event-driven off the session's state stream with their
//! timers gated on activity, so an idle daemon takes no wakeups from them.

use std::sync::Arc;
use std::time::{Duration, Instant};

use api::{
    ApiError, ApiEvent, ConnectKind, ErrorReason, FieldKind, FieldSpec, FieldValue, Icon,
    IntegrationInfo, Phase, PlayerState, Text, value_of,
};
use reader::Track;
use tokio::sync::{broadcast, watch};

use crate::credentials::{CredentialStore, Credentials};
use crate::session::{PlaybackRecorder, SessionHandle};

const DISCORD_APP_ID: &str = "1470087339639443658";
const JELLYFIN_REPORT_SECS: u64 = 5;
const JELLYFIN_KEEPALIVE_TICKS: u32 = 6;
const DISCORD_TICK_SECS: u64 = 30;
const YOUTUBE_REPORT_SECS: u64 = 1;

/// Durable recents + listen counts over the source a track played from,
/// matching the pump: recents record under the DB key, listen counts under the uid.
pub struct SourceRecorder;

#[async_trait::async_trait]
impl PlaybackRecorder for SourceRecorder {
    async fn record_recent(&self, source: Option<&server::source::ActiveSource>, track: &Track) {
        let Some(source) = source else {
            return;
        };
        if let Err(error) = source.record_recent(&track.id.key()).await {
            tracing::warn!(%error, "recent record failed");
        }
    }

    async fn bump_listen_count(
        &self,
        source: Option<&server::source::ActiveSource>,
        track: &Track,
    ) {
        let Some(source) = source else {
            return;
        };
        if let Err(error) = source.bump_listen_count(&track.id.key()).await {
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
    events: &mut broadcast::Receiver<ApiEvent>,
) -> Option<Option<Box<PlayerState>>> {
    match events.recv().await {
        Ok(ApiEvent::PlayerState(state)) => Some(Some(state)),
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
    source: server::source::ActiveSource,
    sources: watch::Receiver<Arc<server::sources::Sources>>,
) -> tokio::task::JoinHandle<()> {
    let mut events = session.subscribe();
    let offline = session.offline().clone();
    tokio::spawn(async move {
        let mut last_id: Option<String> = None;
        let mut last_state: Option<(Box<PlayerState>, Instant)> = None;
        let mut was_playing = false;
        let mut ticks_until_keepalive = 0u32;
        let mut report = tokio::time::interval(Duration::from_secs(JELLYFIN_REPORT_SECS));
        report.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            let jellyfin_configured = {
                let sources = sources.borrow();
                sources
                    .server
                    .as_ref()
                    .is_some_and(|server| server.service == config::MusicService::Jellyfin)
                    && !(offline.offline_mode() && crate::network::needs_internet(&sources))
            };

            tokio::select! {
                state = next_state(&mut events) => {
                    let Some(state) = state else { break };
                    let Some(state) = state else { continue };
                    let received = Instant::now();
                    let current_id = jellyfin_configured
                        .then(|| {
                            state
                                .track
                                .as_ref()
                                .and_then(|track| track.uid.strip_prefix("jellyfin:"))
                                .map(str::to_string)
                        })
                        .flatten();
                    let playing = state.phase == Phase::Playing;
                    let position_ms = interpolated_ms(&state, received);

                    if current_id != last_id {
                        if let Some(old) = last_id.take() {
                            let source = source.clone();
                            let ticks = last_state
                                .as_ref()
                                .map(|(state, received)| interpolated_ms(state, *received))
                                .unwrap_or_default()
                                * 10_000;
                            tokio::spawn(async move {
                                let _ = source.report_playback_stopped(&old, ticks).await;
                            });
                        }
                        if let Some(new) = current_id.clone() {
                            let source = source.clone();
                            tokio::spawn(async move {
                                let _ = source.report_playback_start(&new).await;
                            });
                        }
                        last_id = current_id.clone();
                        ticks_until_keepalive = 0;
                    } else if playing != was_playing
                        && let Some(id) = current_id.clone()
                    {
                        let source = source.clone();
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
                _ = report.tick(), if jellyfin_configured && last_id.is_some() => {
                    let Some(id) = last_id.clone() else { continue };
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

fn youtube_signed_in(sources: &server::sources::Sources) -> bool {
    matches!(sources.active, config::Source::Server(_))
        && sources.server.as_ref().is_some_and(|server| {
            server.service == config::MusicService::YtMusic
                && !server.yt_anonymous
                && server
                    .access_token
                    .as_deref()
                    .is_some_and(|t| !t.is_empty())
        })
}

/// Reports plays to YouTube History while a signed-in YouTube Music source is
/// active and `pause_watch_history` is off. The source is built from the saved
/// sources at the start of each play, so a sign-out or pausing the history stops
/// reporting at the next state change.
pub fn spawn_youtube_reporter(
    session: &SessionHandle,
    db: db::Db,
    sources: watch::Receiver<Arc<server::sources::Sources>>,
    settings: watch::Receiver<config::Settings>,
) -> tokio::task::JoinHandle<()> {
    let mut events = session.subscribe();
    let session = session.clone();
    tokio::spawn(async move {
        let mut current: Option<(String, server::source::ActiveSource)> = None;
        let mut last_state: Option<(Box<PlayerState>, Instant)> = None;
        let mut report = tokio::time::interval(Duration::from_secs(YOUTUBE_REPORT_SECS));
        report.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            let playing = last_state
                .as_ref()
                .is_some_and(|(state, _)| state.phase == Phase::Playing);

            tokio::select! {
                event = events.recv() => {
                    // A missed state can be the one that started the clock.
                    let state = match event {
                        Ok(ApiEvent::PlayerState(state)) => state,
                        Ok(_) => continue,
                        Err(broadcast::error::RecvError::Lagged(_)) => Box::new(session.state()),
                        Err(broadcast::error::RecvError::Closed) => break,
                    };
                    let received = Instant::now();
                    let signed_in = youtube_signed_in(&sources.borrow());
                    let history_paused = settings.borrow().playback.pause_watch_history
                        || session.offline().offline_mode();
                    let live_id = (signed_in
                        && !history_paused
                        && matches!(state.phase, Phase::Playing | Phase::Paused))
                    .then(|| {
                        state
                            .track
                            .as_ref()
                            .and_then(|track| track.uid.strip_prefix("ytmusic:"))
                            .map(str::to_string)
                    })
                    .flatten();

                    if let Some((id, source)) =
                        current.take_if(|(id, _)| live_id.as_deref() != Some(id.as_str()))
                    {
                        // The final pings are what write the history entry.
                        if !history_paused {
                            let ended_ms = last_state
                                .as_ref()
                                .map(|(state, received)| interpolated_ms(state, *received))
                                .unwrap_or_default();
                            let _ = source.report_playback_stopped(&id, ended_ms * 10_000).await;
                        }
                    }
                    if current.is_none()
                        && state.phase == Phase::Playing
                        && let Some(id) = live_id
                    {
                        let source: server::source::ActiveSource =
                            Arc::from(server::source::active(db.clone(), &sources.borrow()));
                        let _ = source.report_playback_start(&id).await;
                        current = Some((id, source));
                    }
                    if let Some((id, source)) = &current {
                        let position_ms = interpolated_ms(&state, received);
                        let paused = state.phase != Phase::Playing;
                        let _ = source
                            .report_playback_progress(id, position_ms * 10_000, paused)
                            .await;
                    }
                    last_state = Some((state, received));
                }
                _ = report.tick(), if playing && current.is_some() => {
                    let (Some((id, source)), Some((state, received))) = (&current, &last_state)
                    else {
                        continue;
                    };
                    let position_ms = interpolated_ms(state, *received);
                    let _ = source
                        .report_playback_progress(id, position_ms * 10_000, false)
                        .await;
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
    mut config: watch::Receiver<config::Settings>,
    sources: watch::Receiver<Arc<server::sources::Sources>>,
    mut paused: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    let mut events = session.subscribe();
    let session = session.clone();
    tokio::spawn(async move {
        let mut presence = None;
        if !hold_connection(
            &mut presence,
            PresenceOptions::of(&config.borrow(), &paused).enabled,
        ) {
            return;
        }
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
        let mut pause_open = true;
        let mut config_open = true;

        loop {
            tokio::select! {
                state = next_state(&mut events) => {
                    let Some(state) = state else { break };
                    let Some(state) = state else { continue };
                    let received = Instant::now();
                    let service = sources.borrow().active_service();
                    let options = PresenceOptions::of(&config.borrow(), &paused);
                    if !present(&mut presence, options, service, &state, received, &mut discord, &session, &cover_tx) {
                        break;
                    }
                    last_state = Some((state, received));
                }
                resolved = cover_rx.recv() => {
                    let Some((song_key, url)) = resolved else { break };
                    if song_key == discord.cover_song_key {
                        discord.cover_task.take();
                        discord.cover = url;
                        discord.cover_sent = false;
                        if let Some((state, received)) = &last_state {
                            let service = sources.borrow().active_service();
                            let options = PresenceOptions::of(&config.borrow(), &paused);
                            if !present(&mut presence, options, service, state, *received, &mut discord, &session, &cover_tx) {
                                break;
                            }
                        }
                    }
                }
                changed = paused.changed(), if pause_open => {
                    // A dropped sender would make every later `changed` return at once.
                    if changed.is_err() {
                        pause_open = false;
                        continue;
                    }
                    if let Some((state, received)) = &last_state {
                        let service = sources.borrow().active_service();
                        let options = PresenceOptions::of(&config.borrow(), &paused);
                        if !present(&mut presence, options, service, state, *received, &mut discord, &session, &cover_tx) {
                            break;
                        }
                    }
                }
                changed = config.changed(), if config_open => {
                    if changed.is_err() {
                        config_open = false;
                        continue;
                    }
                    let options = PresenceOptions::of(&config.borrow(), &paused);
                    if options.enabled == presence.is_some() {
                        continue;
                    }
                    let kept = match &last_state {
                        Some((state, received)) => {
                            let service = sources.borrow().active_service();
                            present(&mut presence, options, service, state, *received, &mut discord, &session, &cover_tx)
                        }
                        None => hold_connection(&mut presence, options.enabled),
                    };
                    if !kept {
                        break;
                    }
                }
                _ = keepalive.tick(), if discord.last_enabled => {
                    if let Some(presence) = &presence {
                        presence.tick();
                    }
                }
            }
        }
    })
}

/// Hold the Discord IPC connection only while presence is enabled; answers false when presence cannot exist on this platform.
fn hold_connection(slot: &mut Option<Arc<discord_presence::Presence>>, enabled: bool) -> bool {
    match (enabled, slot.is_some()) {
        (true, false) => match discord_presence::Presence::new(DISCORD_APP_ID) {
            Ok(presence) => {
                *slot = Some(Arc::new(presence));
                true
            }
            Err(error) => {
                tracing::info!(%error, "discord presence unavailable");
                false
            }
        },
        (false, true) => {
            if let Some(presence) = slot.take()
                && let Err(error) = presence.disconnect()
            {
                tracing::debug!(%error, "closing the discord connection failed");
            }
            true
        }
        _ => true,
    }
}

/// Project `state` over a connection opened for it, clearing the card before a disable closes the connection.
#[allow(clippy::too_many_arguments)]
fn present(
    slot: &mut Option<Arc<discord_presence::Presence>>,
    options: PresenceOptions,
    service: Option<config::MusicService>,
    state: &PlayerState,
    received: Instant,
    discord: &mut DiscordState,
    session: &SessionHandle,
    cover_tx: &tokio::sync::mpsc::UnboundedSender<(String, Option<String>)>,
) -> bool {
    if options.enabled && !hold_connection(slot, true) {
        return false;
    }
    match slot.as_deref() {
        Some(presence) => project(
            presence, options, service, state, received, discord, session, cover_tx,
        ),
        None => {
            discord.cancel_cover_lookup();
            discord.cover_lookup_attempted = false;
            discord.was_playing = state.phase == Phase::Playing;
            discord.last_enabled = false;
        }
    }
    hold_connection(slot, options.enabled)
}

/// What the presence card shows: the two settings, and the pause toggle the daemon keeps beside them.
#[derive(Clone, Copy)]
struct PresenceOptions {
    enabled: bool,
    paused: bool,
    source: bool,
}

impl PresenceOptions {
    fn of(config: &config::Settings, paused: &watch::Receiver<bool>) -> Self {
        Self {
            enabled: config.integrations.discord_presence.unwrap_or(true),
            paused: *paused.borrow(),
            source: config.integrations.discord_presence_source.unwrap_or(true),
        }
    }
}

fn project(
    presence: &discord_presence::Presence,
    options: PresenceOptions,
    service: Option<config::MusicService>,
    state: &PlayerState,
    received: Instant,
    discord: &mut DiscordState,
    session: &SessionHandle,
    cover_tx: &tokio::sync::mpsc::UnboundedSender<(String, Option<String>)>,
) {
    let enabled = options.enabled;
    let paused_enabled = options.paused;
    let source_name = options.source.then(|| {
        service
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
            let session = session.clone();
            let key = track.map(|t| t.key.clone()).unwrap_or_default();
            discord.cover_task = Some(tokio::spawn(async move {
                let track = session.queued_track(&key).await;
                let resolved = match track
                    .as_ref()
                    .and_then(discord_presence::cover_art::youtube_cover_art_url)
                {
                    Some(url) => Some(url),
                    None if session.offline().offline_mode() => None,
                    None => {
                        let mbid = track.and_then(|track| track.musicbrainz_release_id);
                        discord_presence::cover_art::resolve_cover_art_url_cached(
                            mbid.as_deref(),
                            &artist_c,
                            &album_c,
                        )
                        .await
                    }
                };
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

/// The integration ids a client addresses these by.
const DISCORD: &str = "discord";
const LISTENBRAINZ: &str = "listenbrainz";
const LASTFM: &str = "lastfm";
const LIBREFM: &str = "librefm";

/// The field keys a form and its answers agree on.
const PRESENCE: &str = "presence";
const PAUSED: &str = "paused";
const PRESENCE_SOURCE: &str = "source";
const TOKEN: &str = "token";
const API_KEY: &str = "api_key";
const API_SECRET: &str = "api_secret";

fn toggle_field(key: &str, config_key: &str, value: bool) -> FieldSpec {
    FieldSpec {
        key: key.to_string(),
        label: Text::key(config_key),
        kind: FieldKind::Toggle,
        value: Some(value.to_string()),
        config_key: Some(config_key.to_string()),
        ..Default::default()
    }
}

/// A credential row: it carries no value out, only somewhere to type a new one. No settings layer holds a credential, so it has no settings key to lock.
fn secret_field(key: &str, label: Text, placeholder: &str) -> FieldSpec {
    FieldSpec {
        key: key.to_string(),
        label,
        placeholder: Some(Text::key(placeholder)),
        kind: FieldKind::Secret,
        ..Default::default()
    }
}

fn discord(config: &config::Settings, paused: bool) -> IntegrationInfo {
    let presence = config.integrations.discord_presence.unwrap_or(true);
    IntegrationInfo {
        id: DISCORD.to_string(),
        name: Text::key("discord_presence"),
        icon: Icon::Class("fa-brands fa-discord".into()),
        configured: presence,
        connect: ConnectKind::None,
        fields: vec![
            toggle_field(PRESENCE, "discord_presence", presence),
            // Runtime state rather than a setting, so no settings layer can lock it.
            FieldSpec {
                config_key: None,
                ..toggle_field(PAUSED, "discord_presence_paused", paused)
            },
            toggle_field(
                PRESENCE_SOURCE,
                "discord_presence_source",
                config.integrations.discord_presence_source.unwrap_or(true),
            ),
        ],
    }
}

fn listenbrainz(credentials: &Credentials) -> IntegrationInfo {
    IntegrationInfo {
        id: LISTENBRAINZ.to_string(),
        name: Text::key("listenbrainz"),
        icon: Icon::Class("fa-solid fa-compact-disc".into()),
        configured: !credentials.musicbrainz_token.trim().is_empty(),
        // Android has no desktop browser profile to take a session from.
        connect: if cfg!(target_os = "android") {
            ConnectKind::None
        } else {
            ConnectKind::BrowserProfile
        },
        fields: vec![secret_field(
            TOKEN,
            Text::key("listenbrainz"),
            "listenbrainz_token_placeholder",
        )],
    }
}

fn lastfm(credentials: &Credentials) -> IntegrationInfo {
    IntegrationInfo {
        id: LASTFM.to_string(),
        name: Text::key("lastfm"),
        icon: Icon::Class("fa-brands fa-lastfm".into()),
        // A session key is what scrobbles; the api key alone only reaches the sign-in page.
        configured: !credentials.lastfm_session_key.trim().is_empty(),
        connect: ConnectKind::WebSignIn,
        fields: vec![
            secret_field(
                API_KEY,
                Text::key("lastfm_api_key_label"),
                "lastfm_api_key_placeholder",
            ),
            secret_field(
                API_SECRET,
                Text::key("lastfm_api_secret_label"),
                "lastfm_api_secret_placeholder",
            ),
        ],
    }
}

fn librefm(credentials: &Credentials) -> IntegrationInfo {
    IntegrationInfo {
        id: LIBREFM.to_string(),
        name: Text::key("librefm"),
        icon: Icon::Class("fa-brands fa-lastfm".into()),
        configured: !credentials.librefm_session_key.trim().is_empty(),
        connect: ConnectKind::WebSignIn,
        // Its api key and secret are compiled in, so there is nothing to fill in.
        fields: Vec::new(),
    }
}

/// What this build publishes, in the order a settings page lists it.
fn all(config: &config::Settings, credentials: &Credentials, paused: bool) -> Vec<IntegrationInfo> {
    let mut published = Vec::new();
    // Android has no Discord client to talk to, so the row is not offered there.
    if !cfg!(target_os = "android") {
        published.push(discord(config, paused));
    }
    published.push(listenbrainz(credentials));
    published.push(lastfm(credentials));
    published.push(librefm(credentials));
    published
}

fn info_of(
    id: &str,
    config: &config::Settings,
    credentials: &Credentials,
    paused: bool,
) -> Result<IntegrationInfo, ApiError> {
    all(config, credentials, paused)
        .into_iter()
        .find(|info| info.id == id)
        .ok_or_else(|| ApiError::not_found("no such integration"))
}

/// The settings keys an answer set would actually write. A field nobody
/// answered writes nothing, and neither does a secret answered empty.
fn write_keys(info: &IntegrationInfo, values: &[FieldValue]) -> Vec<config::SettingKey> {
    info.fields
        .iter()
        .filter(|field| match value_of(values, &field.key) {
            Some(value) => !matches!(field.kind, FieldKind::Secret) || !value.trim().is_empty(),
            None => false,
        })
        .filter_map(|field| field.config_key.as_deref())
        .filter_map(config::SettingKey::named)
        .collect()
}

/// Keep what is stored unless a new secret was actually typed; emptying a
/// field is not how a credential is dropped.
fn set_secret(values: &[FieldValue], key: &str, stored: &mut String) {
    if let Some(value) = value_of(values, key).map(str::trim)
        && !value.is_empty()
    {
        *stored = value.to_string();
    }
}

fn set_toggle(values: &[FieldValue], key: &str, stored: &mut Option<bool>) {
    if let Some(value) = value_of(values, key) {
        *stored = Some(value == "true");
    }
}

fn apply_presence(values: &[FieldValue], config: &mut config::Settings) {
    set_toggle(values, PRESENCE, &mut config.integrations.discord_presence);
    set_toggle(
        values,
        PRESENCE_SOURCE,
        &mut config.integrations.discord_presence_source,
    );
}

fn apply_credentials(id: &str, values: &[FieldValue], credentials: &mut Credentials) {
    match id {
        LISTENBRAINZ => set_secret(values, TOKEN, &mut credentials.musicbrainz_token),
        LASTFM => {
            set_secret(values, API_KEY, &mut credentials.lastfm_api_key);
            set_secret(values, API_SECRET, &mut credentials.lastfm_api_secret);
        }
        _ => {}
    }
}

/// Clearing drops more than an integration publishes: a session key is held but never rendered.
fn clear_credentials(id: &str, credentials: &mut Credentials) {
    match id {
        LISTENBRAINZ => credentials.musicbrainz_token.clear(),
        LASTFM => {
            credentials.lastfm_api_key.clear();
            credentials.lastfm_api_secret.clear();
            credentials.lastfm_session_key.clear();
        }
        LIBREFM => credentials.librefm_session_key.clear(),
        _ => {}
    }
}

fn database_error(error: db::DbError) -> ApiError {
    ApiError::internal(format!("database error: {error}"))
}

/// Whether Discord presence stays up while playback is paused; stored before the presence task sees it.
#[derive(Clone)]
pub struct PresencePause {
    db: db::Db,
    held: Arc<watch::Sender<bool>>,
    writer: Arc<tokio::sync::Mutex<()>>,
}

impl PresencePause {
    pub async fn load(db: &db::Db) -> Result<Self, db::DbError> {
        let paused = db.discord_presence_paused().await?.unwrap_or(true);
        Ok(Self {
            db: db.clone(),
            held: Arc::new(watch::channel(paused).0),
            writer: Arc::new(tokio::sync::Mutex::new(())),
        })
    }

    pub fn get(&self) -> bool {
        *self.held.borrow()
    }

    pub fn watch(&self) -> watch::Receiver<bool> {
        self.held.subscribe()
    }

    pub async fn set(&self, paused: bool) -> Result<(), db::DbError> {
        let _writing = self.writer.lock().await;
        self.db.set_discord_presence_paused(paused).await?;
        self.held.send_replace(paused);
        Ok(())
    }
}

/// Scrobbling, metadata and presence accounts.
///
/// The credentials for these lived in the settings UI, which meant a Last.fm
/// api secret sat in a Dioxus signal and the scrobbler that used it ran in the
/// frontend. They are write-only here: a caller answers a published field list
/// and asks whether one is configured, never what it holds.
pub struct IntegrationService {
    config: Arc<crate::config_service::ConfigService>,
    credentials: CredentialStore,
    presence_pause: PresencePause,
}

impl IntegrationService {
    pub fn new(
        config: Arc<crate::config_service::ConfigService>,
        credentials: CredentialStore,
        presence_pause: PresencePause,
    ) -> Arc<Self> {
        Arc::new(Self {
            config,
            credentials,
            presence_pause,
        })
    }

    pub async fn list(&self) -> Vec<IntegrationInfo> {
        all(
            &self.config.snapshot().await,
            &self.credentials.snapshot(),
            self.presence_pause.get(),
        )
    }

    async fn info(&self, id: &str) -> Result<IntegrationInfo, ApiError> {
        info_of(
            id,
            &self.config.snapshot().await,
            &self.credentials.snapshot(),
            self.presence_pause.get(),
        )
    }

    /// Answer an integration's fields. An absent key is left alone, and so is
    /// a secret answered empty.
    pub async fn set_settings(
        &self,
        id: &str,
        values: Vec<FieldValue>,
    ) -> Result<IntegrationInfo, ApiError> {
        let info = self.info(id).await?;
        self.config
            .ensure_unlocked(&write_keys(&info, &values))
            .await?;
        if id == DISCORD {
            if let Some(paused) = value_of(&values, PAUSED) {
                self.presence_pause
                    .set(paused == "true")
                    .await
                    .map_err(database_error)?;
            }
            if [PRESENCE, PRESENCE_SOURCE]
                .iter()
                .any(|key| value_of(&values, key).is_some())
            {
                self.config
                    .mutate_state(&PRESENCE_KEYS, move |config| {
                        apply_presence(&values, config)
                    })
                    .await?;
            }
        } else {
            let target = id.to_string();
            self.credentials
                .update(move |credentials| apply_credentials(&target, &values, credentials))
                .await
                .map_err(database_error)?;
        }
        self.info(id).await
    }

    pub async fn clear(&self, id: &str) -> Result<(), ApiError> {
        self.info(id).await?;
        if id == DISCORD {
            self.config.ensure_unlocked(&PRESENCE_KEYS[..1]).await?;
            // Discord holds no credential, so unconfigured means presence off.
            self.config
                .mutate_state(&PRESENCE_KEYS[..1], |config| {
                    config.integrations.discord_presence = Some(false)
                })
                .await?;
        } else {
            let target = id.to_string();
            self.credentials
                .update(move |credentials| clear_credentials(&target, credentials))
                .await
                .map_err(database_error)?;
        }
        Ok(())
    }

    /// Run a service's web sign-in and keep the session key it returns.
    ///
    /// Last.fm and Libre.fm both hand out a token, open a page for the person
    /// to approve, then trade the token for a session key -- a browser and a
    /// poll loop, which is why it is not a frontend's job.
    pub async fn authenticate(&self, id: &str) -> Result<IntegrationInfo, ApiError> {
        let info = self.info(id).await?;
        if info.connect != ConnectKind::WebSignIn {
            return Err(ApiError::unsupported(
                "this integration does not connect with a web sign-in",
            )
            .because(ErrorReason::ConnectNotOffered));
        }
        let librefm = id == LIBREFM;
        let (api_key, api_secret) = if librefm {
            (
                scrobble::librefm::API_KEY.to_string(),
                scrobble::librefm::API_SECRET.to_string(),
            )
        } else {
            let held = self.credentials.snapshot();
            (held.lastfm_api_key, held.lastfm_api_secret)
        };
        if api_key.trim().is_empty() || api_secret.trim().is_empty() {
            return Err(ApiError::invalid_input("set the API key and secret first")
                .because(ErrorReason::ApiKeyMissing));
        }
        let session_key = web_sign_in(id, &api_key, &api_secret).await?;
        self.credentials
            .update(move |credentials| {
                if librefm {
                    credentials.librefm_session_key = session_key;
                } else {
                    credentials.lastfm_session_key = session_key;
                }
            })
            .await
            .map_err(database_error)?;
        self.info(id).await
    }

    /// Take an integration's credential from one of [`browser_profiles`] and
    /// keep it once the service accepts it. `profile` must be one of those,
    /// so a client cannot point the daemon at any other directory.
    pub async fn connect_from_browser(
        &self,
        id: &str,
        profile: &str,
    ) -> Result<IntegrationInfo, ApiError> {
        let info = self.info(id).await?;
        // ListenBrainz is the only one offering it, so the token is its.
        if id != LISTENBRAINZ || info.connect != ConnectKind::BrowserProfile {
            return Err(ApiError::unsupported(
                "this integration does not connect from a browser profile",
            )
            .because(ErrorReason::ConnectNotOffered));
        }
        let profile = server::cookies::browser_profiles()
            .into_iter()
            .find(|candidate| candidate.dir.to_string_lossy() == profile)
            .ok_or_else(|| {
                ApiError::not_found("that browser profile is no longer there")
                    .because(ErrorReason::BrowserProfileGone)
            })?;
        let token = listenbrainz_token_from(&profile).await?;
        let user = scrobble::listenbrainz::validate_token(&token)
            .await
            .map_err(|error| {
                ApiError::internal(format!("could not reach ListenBrainz: {error}"))
                    .because(ErrorReason::IntegrationUnreachable)
            })?
            .ok_or_else(|| {
                ApiError::invalid_input(
                    "ListenBrainz does not recognise the token from that profile. Paste your token instead.",
                )
                .because(ErrorReason::CredentialRejected)
            })?;
        tracing::info!(%user, browser = profile.browser.id(), "connected to listenbrainz from a browser");
        self.credentials
            .update(move |credentials| credentials.musicbrainz_token = token)
            .await
            .map_err(database_error)?;
        self.info(id).await
    }
}

/// The person's own browser profiles, keyed by their directory.
pub fn browser_profiles() -> Vec<api::BrowserProfile> {
    server::cookies::browser_profiles()
        .into_iter()
        .map(|profile| api::BrowserProfile {
            id: profile.dir.to_string_lossy().into_owned(),
            browser: profile.browser.label().to_string(),
            name: profile.name,
        })
        .collect()
}

/// The ListenBrainz user token of the session in a signed-in browser profile.
async fn listenbrainz_token_from(
    profile: &server::cookies::BrowserProfile,
) -> Result<String, ApiError> {
    let cookies = server::cookies::read_cookies(profile.browser, &profile.dir, "listenbrainz.org")
        .await
        .map_err(|error| {
            ApiError::internal(format!(
                "could not read the cookies of {} ({}): {error}",
                profile.name,
                profile.browser.label()
            ))
            .because(ErrorReason::BrowserCookiesUnreadable)
        })?;
    let header = scrobble::listenbrainz::session_header(
        cookies.iter().map(|c| (c.name.as_str(), c.value.as_str())),
    )
    .ok_or_else(|| {
        ApiError::invalid_input(format!(
            "{} ({}) is not signed in to listenbrainz.org. Sign in there, or paste your token instead.",
            profile.name,
            profile.browser.label()
        ))
        .because(ErrorReason::BrowserProfileSignedOut)
    })?;
    scrobble::listenbrainz::token_from_session(&header)
        .await
        .ok_or_else(|| {
            ApiError::invalid_input(
                "listenbrainz.org did not show a token for that profile. Paste it from listenbrainz.org/settings instead.",
            )
            .because(ErrorReason::BrowserProfileNoToken)
        })
}

/// What a presence write reports as changed, so the session republishes the config.
const PRESENCE_KEYS: [config::SettingKey; 2] = [
    config::SettingKey::Integrations(config::IntegrationsKey::DiscordPresence),
    config::SettingKey::Integrations(config::IntegrationsKey::DiscordPresenceSource),
];

/// Open the approval page, then poll for the session key. The person has to
/// click through in a browser, so the wait is generous and bounded.
#[cfg(not(target_os = "android"))]
async fn web_sign_in(id: &str, api_key: &str, api_secret: &str) -> Result<String, ApiError> {
    let librefm = id == LIBREFM;
    let token = if librefm {
        scrobble::librefm::get_auth_token(api_key).await
    } else {
        scrobble::lastfm::get_auth_token(api_key).await
    }
    .map_err(|error| {
        ApiError::internal(format!("could not start the sign-in: {error}"))
            .because(ErrorReason::SignInNotStarted)
    })?;
    let url = if librefm {
        scrobble::librefm::auth_url(api_key, &token)
    } else {
        scrobble::lastfm::auth_url(api_key, &token)
    };
    webbrowser::open(&url).map_err(|error| {
        ApiError::internal(format!("could not open a browser: {error}"))
            .because(ErrorReason::BrowserNotOpened)
    })?;
    for _ in 0..60 {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        let session = if librefm {
            scrobble::librefm::get_session_key(api_key, api_secret, &token).await
        } else {
            scrobble::lastfm::get_session_key(api_key, api_secret, &token).await
        };
        if let Ok(session) = session {
            return Ok(session);
        }
    }
    Err(ApiError::internal("the sign-in was not approved in time")
        .because(ErrorReason::SignInTimedOut))
}

#[cfg(target_os = "android")]
async fn web_sign_in(_id: &str, _api_key: &str, _api_secret: &str) -> Result<String, ApiError> {
    Err(
        ApiError::unsupported("web sign-in runs in the app on Android")
            .because(ErrorReason::ConnectNotOffered),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn youtube(token: &str, anonymous: bool) -> server::sources::Sources {
        let mut sources = server::sources::Sources {
            active: config::Source::Server("yt".into()),
            ..Default::default()
        };
        sources.server = Some(config::MusicServer {
            service: config::MusicService::YtMusic,
            access_token: Some(token.into()),
            yt_anonymous: anonymous,
            ..config::MusicServer::new("YouTube Music".into(), String::new())
        });
        sources
    }

    #[test]
    fn only_a_signed_in_youtube_source_reports_plays() {
        assert!(youtube_signed_in(&youtube("SAPISID=abc", false)));
        assert!(!youtube_signed_in(&youtube("", true)));
        assert!(!youtube_signed_in(&youtube("", false)));
        let mut local = youtube("SAPISID=abc", false);
        local.active = config::Source::LocalLibrary("local".into());
        assert!(!youtube_signed_in(&local));
    }

    fn answer(key: &str, value: &str) -> FieldValue {
        FieldValue {
            key: key.to_string(),
            value: value.to_string(),
        }
    }

    #[test]
    fn a_disabled_presence_opens_no_connection() {
        let mut slot = None;
        assert!(hold_connection(&mut slot, false));
        assert!(
            slot.is_none(),
            "nothing dials Discord while presence is off"
        );
    }

    #[tokio::test]
    async fn a_pause_toggle_reaches_the_presence_task_and_the_database() {
        let dir = tempfile::tempdir().expect("tempdir");
        let database = db::init(&dir.path().join("presence.db")).await.expect("db");
        let config = Arc::new(crate::config_service::ConfigService::new(
            database.clone(),
            dir.path().join("settings.toml"),
            config::Settings::default(),
        ));
        let credentials = CredentialStore::load(&database).await.expect("credentials");
        let pause = PresencePause::load(&database).await.expect("pause");
        assert!(pause.get(), "presence stays up while paused by default");
        let mut presence_task = pause.watch();
        let service = IntegrationService::new(config.clone(), credentials, pause.clone());

        let info = service
            .set_settings(DISCORD, vec![answer(PAUSED, "false")])
            .await
            .expect("toggle");

        presence_task
            .changed()
            .await
            .expect("the presence task hears it");
        assert!(!*presence_task.borrow());
        let field = info
            .fields
            .iter()
            .find(|field| field.key == PAUSED)
            .expect("paused field");
        assert_eq!(field.value.as_deref(), Some("false"));
        assert_eq!(field.config_key, None);
        assert_eq!(
            database.discord_presence_paused().await.expect("stored"),
            Some(false)
        );
        let reloaded = PresencePause::load(&database).await.expect("reload");
        assert!(!reloaded.get(), "the toggle survives a restart");
        assert_eq!(
            config.snapshot().await.integrations.discord_presence,
            Some(true),
            "the presence setting is left alone"
        );
    }

    #[tokio::test]
    async fn only_a_web_sign_in_integration_starts_a_web_sign_in() {
        let dir = tempfile::tempdir().expect("tempdir");
        let database = db::init(&dir.path().join("sign-in.db")).await.expect("db");
        let config = Arc::new(crate::config_service::ConfigService::new(
            database.clone(),
            dir.path().join("settings.toml"),
            config::Settings::default(),
        ));
        let credentials = CredentialStore::load(&database).await.expect("credentials");
        let pause = PresencePause::load(&database).await.expect("pause");
        let service = IntegrationService::new(config, credentials, pause);

        for id in [LISTENBRAINZ, DISCORD] {
            let refused = service
                .authenticate(id)
                .await
                .expect_err("no web sign-in to run");
            assert_eq!(refused.code, api::ErrorCode::Unsupported, "{id}");
            assert_eq!(refused.reason, Some(ErrorReason::ConnectNotOffered), "{id}");
        }
        let keyless = service
            .authenticate(LASTFM)
            .await
            .expect_err("no API key to sign in with");
        assert_eq!(keyless.reason, Some(ErrorReason::ApiKeyMissing));
        let unknown = service
            .authenticate("nope")
            .await
            .expect_err("unknown integration");
        assert_eq!(unknown.code, api::ErrorCode::NotFound);
    }
}
