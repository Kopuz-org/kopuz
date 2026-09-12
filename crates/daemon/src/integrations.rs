//! Playback integrations ported from the event pump: the Jellyfin session
//! reporter and the Discord presence projector, plus the source-backed
//! [`PlaybackRecorder`].
//!
//! Both tasks are event-driven off the session's state stream with their
//! timers gated on activity, so an idle daemon takes no wakeups from them.

use std::sync::Arc;
use std::time::{Duration, Instant};

use api::{
    ApiError, ApiEvent, ConnectKind, FieldKind, FieldSpec, FieldValue, Icon, IntegrationInfo,
    Phase, PlayerState, Text, value_of,
};
use reader::Track;
use tokio::sync::{broadcast, watch};

use crate::session::{PlaybackRecorder, SessionHandle};

const DISCORD_APP_ID: &str = "1470087339639443658";
const JELLYFIN_REPORT_SECS: u64 = 5;
const JELLYFIN_KEEPALIVE_TICKS: u32 = 6;
const DISCORD_TICK_SECS: u64 = 30;

/// Durable recents + listen counts over the active source, matching the
/// pump: recents record under the DB key, listen counts under the uid.
pub struct SourceRecorder {
    source: server::source::ActiveSource,
}

impl SourceRecorder {
    pub fn new(source: server::source::ActiveSource) -> Self {
        Self { source }
    }
}

#[async_trait::async_trait]
impl PlaybackRecorder for SourceRecorder {
    async fn record_recent(&self, track: &Track) {
        if let Err(error) = self.source.record_recent(&track.id.key()).await {
            tracing::warn!(%error, "recent record failed");
        }
    }

    async fn bump_listen_count(&self, track: &Track) {
        if let Err(error) = self.source.bump_listen_count(&track.id.uid()).await {
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
    config: watch::Receiver<config::AppConfig>,
) -> tokio::task::JoinHandle<()> {
    let mut events = session.subscribe();
    tokio::spawn(async move {
        let mut last_id: Option<String> = None;
        let mut last_state: Option<(Box<PlayerState>, Instant)> = None;
        let mut was_playing = false;
        let mut ticks_until_keepalive = 0u32;
        let mut report = tokio::time::interval(Duration::from_secs(JELLYFIN_REPORT_SECS));
        report.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            let jellyfin_configured = config
                .borrow()
                .server
                .as_ref()
                .is_some_and(|server| server.service == config::MusicService::Jellyfin);

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
    let session = session.clone();
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
                    project(&presence, &config.borrow(), &state, received, &mut discord, &session, &cover_tx);
                    last_state = Some((state, received));
                }
                resolved = cover_rx.recv() => {
                    let Some((song_key, url)) = resolved else { break };
                    if song_key == discord.cover_song_key {
                        discord.cover_task.take();
                        discord.cover = url;
                        discord.cover_sent = false;
                        if let Some((state, received)) = &last_state {
                            project(&presence, &config.borrow(), state, *received, &mut discord, &session, &cover_tx);
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
    session: &SessionHandle,
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
            let session = session.clone();
            let key = track.map(|t| t.key.clone()).unwrap_or_default();
            discord.cover_task = Some(tokio::spawn(async move {
                let track = session.queued_track(&key).await;
                let resolved = match track
                    .as_ref()
                    .and_then(discord_presence::cover_art::youtube_cover_art_url)
                {
                    Some(url) => Some(url),
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

/// A credential row: it carries no value out, only somewhere to type a new one.
fn secret_field(key: &str, label: Text, placeholder: &str, config_key: &str) -> FieldSpec {
    FieldSpec {
        key: key.to_string(),
        label,
        placeholder: Some(Text::key(placeholder)),
        kind: FieldKind::Secret,
        config_key: Some(config_key.to_string()),
        ..Default::default()
    }
}

fn discord(config: &config::AppConfig) -> IntegrationInfo {
    let presence = config.discord_presence.unwrap_or(true);
    IntegrationInfo {
        id: DISCORD.to_string(),
        name: Text::key("discord_presence"),
        icon: Icon::Class("fa-brands fa-discord".into()),
        configured: presence,
        connect: ConnectKind::None,
        fields: vec![
            toggle_field(PRESENCE, "discord_presence", presence),
            toggle_field(
                PAUSED,
                "discord_presence_paused",
                config.discord_presence_paused.unwrap_or(true),
            ),
            toggle_field(
                PRESENCE_SOURCE,
                "discord_presence_source",
                config.discord_presence_source.unwrap_or(true),
            ),
        ],
    }
}

fn listenbrainz(config: &config::AppConfig) -> IntegrationInfo {
    IntegrationInfo {
        id: LISTENBRAINZ.to_string(),
        name: Text::key("listenbrainz"),
        icon: Icon::Class("fa-solid fa-compact-disc".into()),
        configured: !config.musicbrainz_token.trim().is_empty(),
        connect: ConnectKind::None,
        fields: vec![secret_field(
            TOKEN,
            Text::key("listenbrainz"),
            "listenbrainz_token_placeholder",
            "musicbrainz_token",
        )],
    }
}

fn lastfm(config: &config::AppConfig) -> IntegrationInfo {
    IntegrationInfo {
        id: LASTFM.to_string(),
        name: Text::key("lastfm"),
        icon: Icon::Class("fa-brands fa-lastfm".into()),
        // A session key is what scrobbles; the api key alone only reaches the sign-in page.
        configured: !config.lastfm_session_key.trim().is_empty(),
        connect: ConnectKind::WebSignIn,
        fields: vec![
            secret_field(
                API_KEY,
                Text::key("lastfm_api_key_label"),
                "lastfm_api_key_placeholder",
                "lastfm_api_key",
            ),
            secret_field(
                API_SECRET,
                Text::key("lastfm_api_secret_label"),
                "lastfm_api_secret_placeholder",
                "lastfm_api_secret",
            ),
        ],
    }
}

fn librefm(config: &config::AppConfig) -> IntegrationInfo {
    IntegrationInfo {
        id: LIBREFM.to_string(),
        name: Text::key("librefm"),
        icon: Icon::Class("fa-brands fa-lastfm".into()),
        configured: !config.librefm_session_key.trim().is_empty(),
        connect: ConnectKind::WebSignIn,
        // Its api key and secret are compiled in, so there is nothing to fill in.
        fields: Vec::new(),
    }
}

/// What this build publishes, in the order a settings page lists it.
fn all(config: &config::AppConfig) -> Vec<IntegrationInfo> {
    let mut published = Vec::new();
    // Android has no Discord client to talk to, so the row is not offered there.
    if !cfg!(target_os = "android") {
        published.push(discord(config));
    }
    published.push(listenbrainz(config));
    published.push(lastfm(config));
    published.push(librefm(config));
    published
}

fn info_of(id: &str, config: &config::AppConfig) -> Result<IntegrationInfo, ApiError> {
    all(config)
        .into_iter()
        .find(|info| info.id == id)
        .ok_or_else(|| ApiError::not_found("no such integration"))
}

/// The settings keys an answer set would actually write. A field nobody
/// answered writes nothing, and neither does a secret answered empty.
fn write_keys<'a>(info: &'a IntegrationInfo, values: &[FieldValue]) -> Vec<&'a str> {
    info.fields
        .iter()
        .filter(|field| match value_of(values, &field.key) {
            Some(value) => !matches!(field.kind, FieldKind::Secret) || !value.trim().is_empty(),
            None => false,
        })
        .filter_map(|field| field.config_key.as_deref())
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

fn apply(id: &str, values: &[FieldValue], config: &mut config::AppConfig) {
    match id {
        DISCORD => {
            set_toggle(values, PRESENCE, &mut config.discord_presence);
            set_toggle(values, PAUSED, &mut config.discord_presence_paused);
            set_toggle(values, PRESENCE_SOURCE, &mut config.discord_presence_source);
        }
        LISTENBRAINZ => set_secret(values, TOKEN, &mut config.musicbrainz_token),
        LASTFM => {
            set_secret(values, API_KEY, &mut config.lastfm_api_key);
            set_secret(values, API_SECRET, &mut config.lastfm_api_secret);
        }
        _ => {}
    }
}

/// The keys clearing one writes, which is more than it publishes: a session
/// key is held but never rendered.
fn clear_keys(id: &str) -> Vec<&'static str> {
    match id {
        DISCORD => vec!["discord_presence"],
        LISTENBRAINZ => vec!["musicbrainz_token"],
        LASTFM => vec!["lastfm_api_key", "lastfm_api_secret", "lastfm_session_key"],
        LIBREFM => vec!["librefm_session_key"],
        _ => Vec::new(),
    }
}

fn clear_stored(id: &str, config: &mut config::AppConfig) {
    match id {
        // Discord holds no credential, so unconfigured means presence off.
        DISCORD => config.discord_presence = Some(false),
        LISTENBRAINZ => config.musicbrainz_token.clear(),
        LASTFM => {
            config.lastfm_api_key.clear();
            config.lastfm_api_secret.clear();
            config.lastfm_session_key.clear();
        }
        LIBREFM => config.librefm_session_key.clear(),
        _ => {}
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
    session: crate::session::SessionHandle,
}

impl IntegrationService {
    pub fn new(
        config: Arc<crate::config_service::ConfigService>,
        session: crate::session::SessionHandle,
    ) -> Arc<Self> {
        Arc::new(Self { config, session })
    }

    pub async fn list(&self) -> Vec<IntegrationInfo> {
        all(&self.config.snapshot().await)
    }

    /// Answer an integration's fields. An absent key is left alone, and so is
    /// a secret answered empty.
    pub async fn set_settings(
        &self,
        id: &str,
        values: Vec<FieldValue>,
    ) -> Result<IntegrationInfo, ApiError> {
        let info = info_of(id, &self.config.snapshot().await)?;
        self.config.ensure_unlocked(&write_keys(&info, &values))?;
        let target = id.to_string();
        let updated = self
            .config
            .mutate_state(move |config| apply(&target, &values, config))
            .await?;
        let info = info_of(id, &updated)?;
        self.publish(updated);
        Ok(info)
    }

    pub async fn clear(&self, id: &str) -> Result<(), ApiError> {
        info_of(id, &self.config.snapshot().await)?;
        self.config.ensure_unlocked(&clear_keys(id))?;
        let target = id.to_string();
        let updated = self
            .config
            .mutate_state(move |config| clear_stored(&target, config))
            .await?;
        self.publish(updated);
        Ok(())
    }

    /// Run a service's web sign-in and keep the session key it returns.
    ///
    /// Last.fm and Libre.fm both hand out a token, open a page for the person
    /// to approve, then trade the token for a session key -- a browser and a
    /// poll loop, which is why it is not a frontend's job.
    pub async fn authenticate(&self, id: &str) -> Result<IntegrationInfo, ApiError> {
        let config = self.config.snapshot().await;
        let info = info_of(id, &config)?;
        if matches!(info.connect, ConnectKind::None) {
            return Err(ApiError::unsupported(
                "this integration takes a token rather than a web sign-in",
            ));
        }
        let librefm = id == LIBREFM;
        let session_setting = if librefm {
            "librefm_session_key"
        } else {
            "lastfm_session_key"
        };
        self.config.ensure_unlocked(&[session_setting])?;
        let (api_key, api_secret) = if librefm {
            (
                scrobble::librefm::API_KEY.to_string(),
                scrobble::librefm::API_SECRET.to_string(),
            )
        } else {
            (
                config.lastfm_api_key.clone(),
                config.lastfm_api_secret.clone(),
            )
        };
        if api_key.trim().is_empty() || api_secret.trim().is_empty() {
            return Err(ApiError::invalid_input("set the API key and secret first"));
        }
        let session_key = web_sign_in(id, &api_key, &api_secret).await?;
        let updated = self
            .config
            .mutate_state(move |config| {
                if librefm {
                    config.librefm_session_key = session_key;
                } else {
                    config.lastfm_session_key = session_key;
                }
            })
            .await?;
        let info = info_of(id, &updated)?;
        self.publish(updated);
        Ok(info)
    }

    /// Push the change so the running scrobbler picks it up.
    fn publish(&self, updated: config::AppConfig) {
        self.session
            .set_config(updated, vec!["integrations".to_string()]);
    }
}

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
    .map_err(|error| ApiError::internal(error.to_string()))?;
    let url = if librefm {
        scrobble::librefm::auth_url(api_key, &token)
    } else {
        scrobble::lastfm::auth_url(api_key, &token)
    };
    webbrowser::open(&url)
        .map_err(|error| ApiError::internal(format!("could not open a browser: {error}")))?;
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
    Err(ApiError::internal("the sign-in was not approved in time"))
}

#[cfg(target_os = "android")]
async fn web_sign_in(_id: &str, _api_key: &str, _api_secret: &str) -> Result<String, ApiError> {
    Err(ApiError::unsupported(
        "web sign-in runs in the app on Android",
    ))
}
