#[cfg(any(target_os = "linux", target_os = "android"))]
use crate::use_player_controller::LoopMode;
use crate::use_player_controller::PlayerController;
use config::AppConfig;
use config::MusicService;
use dioxus::logger::tracing::Instrument;
use dioxus::prelude::*;
use std::sync::Arc;

use discord_presence::Presence;
use discord_presence::cover_art;

#[path = "player_task_event_pump.rs"]
mod event_pump;

#[cfg(target_os = "macos")]
use player::systemint::set_background_handler;

#[cfg(target_os = "macos")]
use player::systemint::set_tokio_waker;

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
enum BgCmd {
    Play,
    Pause,
    Toggle,
    Next,
    Prev,
    Seek(f64),
    #[cfg(target_os = "android")]
    ToggleShuffle,
    #[cfg(target_os = "android")]
    CycleRepeat,
}

static BG_CMD_TX: std::sync::OnceLock<std::sync::Mutex<std::sync::mpsc::Sender<BgCmd>>> =
    std::sync::OnceLock::new();
static BG_CMD_RX: std::sync::OnceLock<std::sync::Mutex<std::sync::mpsc::Receiver<BgCmd>>> =
    std::sync::OnceLock::new();
static BG_NOTIFY: std::sync::OnceLock<tokio::sync::Notify> = std::sync::OnceLock::new();

/// Persist a play-count increment as a single-row upsert. The in-memory
/// `config.listen_counts` is bumped by the caller for live views; this is the
/// durable side (no whole-config rewrite on the play hot path).
fn bump_listen_count_db(track_uid: String, source: ::server::source::ActiveSource) {
    spawn(async move {
        if let Err(e) = source.bump_listen_count(&track_uid).await {
            tracing::warn!(error = %e, "listen count persist failed");
        }
    });
}

#[cfg(any(target_os = "macos", target_os = "android"))]
fn init_bg_channel() {
    BG_CMD_TX.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<BgCmd>();
        let _ = BG_CMD_RX.set(std::sync::Mutex::new(rx));
        std::sync::Mutex::new(tx)
    });
    BG_NOTIFY.get_or_init(tokio::sync::Notify::new);
}

#[allow(dead_code)]
fn send_bg_cmd(cmd: BgCmd) {
    if let Some(lock) = BG_CMD_TX.get()
        && let Ok(tx) = lock.lock()
    {
        let _ = tx.send(cmd);
    }
    // Instantly wake the tokio task so it processes the command
    // without waiting for the next 250ms poll tick.
    if let Some(notify) = BG_NOTIFY.get() {
        notify.notify_one();
    }
}

/// Record one completed listen for the current track — the same accounting the
/// engine's completion path does, used by the external (Spotify) advance paths.
fn record_listen(mut config: Signal<AppConfig>, ctrl: &PlayerController) {
    let idx = *ctrl.current_queue_index.peek();
    if let Some(track) = ctrl.get_track_at(idx) {
        let track_id = track.id.uid().to_string();
        let source = ctrl.active_source.peek().clone();
        let count_key = source.source().listen_count_key(&track_id);
        *config.write().listen_counts.entry(count_key).or_insert(0) += 1;
        bump_listen_count_db(track_id, source);
    }
}

/// The now-playing fields the OS media widget needs, cloned out of the Dioxus
/// signals by the caller so this seam stays UI-framework-free.
struct OsTrack<'a> {
    title: &'a str,
    artist: &'a str,
    album: &'a str,
    duration_secs: u64,
    position_secs: u64,
    playing: bool,
    artwork: Option<&'a str>,
}

/// Platforms whose OS media widget Kopuz drives directly. Matches the set the
/// local `Player` pushes to; for remote Spotify playback the engine is stopped,
/// so the poll loop feeds these the now-playing metadata itself.
#[cfg(any(
    target_os = "macos",
    target_os = "linux",
    target_os = "windows",
    target_os = "android"
))]
mod os_now_playing {
    use super::OsTrack;

    /// The OS media widget wants a local file, not a URL. Cache `cover_url` to a
    /// temp file keyed by the (base62, filename-safe) Spotify track id, so the
    /// same track reuses the file and the macOS artwork cache stays warm.
    /// Returns `None` on empty url or any IO/network failure — a missing cover
    /// just shows the widget's generic glyph.
    pub(super) async fn cache_artwork(track_id: &str, cover_url: &str) -> Option<String> {
        if cover_url.is_empty() {
            return None;
        }
        let path = std::env::temp_dir().join(format!("kopuz_remote_cover_{track_id}.jpg"));
        if !path.exists() {
            let bytes = reqwest::get(cover_url).await.ok()?.bytes().await.ok()?;
            tokio::fs::write(&path, bytes).await.ok()?;
        }
        Some(path.to_string_lossy().into_owned())
    }

    /// Push an external (Spotify Connect) track to the OS media widget. The local
    /// engine is stopped for remote playback, so nothing else feeds it.
    pub(super) fn push(track: OsTrack<'_>) {
        player::systemint::update_now_playing(
            track.title,
            track.artist,
            track.album,
            track.duration_secs as f64,
            track.position_secs as f64,
            track.playing,
            track.artwork,
        );
    }
}

#[cfg(not(any(
    target_os = "macos",
    target_os = "linux",
    target_os = "windows",
    target_os = "android"
)))]
mod os_now_playing {
    use super::OsTrack;

    pub(super) async fn cache_artwork(_track_id: &str, _cover_url: &str) -> Option<String> {
        None
    }

    pub(super) fn push(_track: OsTrack<'_>) {}
}

fn drain_bg_cmds() -> Vec<BgCmd> {
    let mut cmds = Vec::new();
    if let Some(lock) = BG_CMD_RX.get()
        && let Ok(rx) = lock.try_lock()
    {
        while let Ok(cmd) = rx.try_recv() {
            cmds.push(cmd);
        }
    }
    cmds
}

#[inline]
fn nudge_event_loop() {
    #[cfg(target_os = "macos")]
    player::systemint::wake_run_loop();
}

pub fn use_player_task(ctrl: PlayerController) {
    let config: Signal<AppConfig> = use_context();
    #[cfg(target_os = "macos")]
    use_hook(move || {
        init_bg_channel();

        // let the CFRunLoopTimer heartbeat poke our tokio task so it
        // doesn't stall when macOS coalesces tokio::time::sleep
        set_tokio_waker(|| {
            if let Some(notify) = BG_NOTIFY.get() {
                notify.notify_one();
            }
        });

        set_background_handler(move |event| {
            use player::systemint::SystemEvent;
            let cmd = match event {
                SystemEvent::Play => BgCmd::Play,
                SystemEvent::Pause => BgCmd::Pause,
                SystemEvent::Toggle => BgCmd::Toggle,
                SystemEvent::Next => BgCmd::Next,
                SystemEvent::Prev => BgCmd::Prev,
                SystemEvent::Seek(secs) => BgCmd::Seek(secs),
            };
            send_bg_cmd(cmd);
            nudge_event_loop();
        });
    });

    // Keep MPRIS / the Android notification in sync with the UI's own toggles.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    use_effect(move || {
        let shuffle = *ctrl.shuffle.read();
        let repeat = match *ctrl.loop_mode.read() {
            LoopMode::None => player::systemint::RepeatMode::Off,
            LoopMode::Queue => player::systemint::RepeatMode::Playlist,
            LoopMode::Track => player::systemint::RepeatMode::Track,
        };
        player::systemint::update_modes(shuffle, repeat);
    });

    #[cfg(target_os = "linux")]
    use_future(move || {
        let mut ctrl = ctrl;
        async move {
            use player::systemint::{RepeatMode, SystemEvent, poll_event};
            loop {
                let mut processed = false;
                while let Some(event) = poll_event() {
                    processed = true;
                    match event {
                        SystemEvent::Play => ctrl.resume(),
                        SystemEvent::Pause => ctrl.pause(),
                        SystemEvent::Toggle => ctrl.toggle(),
                        SystemEvent::Next => ctrl.play_next(),
                        SystemEvent::Prev => ctrl.play_prev(),
                        SystemEvent::Seek(secs) => {
                            ctrl.seek(std::time::Duration::from_secs_f64(secs));
                        }
                        SystemEvent::SetShuffle(on) => ctrl.set_shuffle(on),
                        SystemEvent::SetRepeat(mode) => ctrl.set_loop_mode(match mode {
                            RepeatMode::Off => LoopMode::None,
                            RepeatMode::Playlist => LoopMode::Queue,
                            RepeatMode::Track => LoopMode::Track,
                        }),
                    }
                }
                if !processed {
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
            }
        }
    });

    #[cfg(target_os = "windows")]
    use_future(move || {
        let mut ctrl = ctrl;
        async move {
            use player::systemint::{SystemEvent, wait_event};
            player::systemint::init();
            tracing::debug!("starting Windows SMTC event loop");
            loop {
                match wait_event().await {
                    Some(SystemEvent::Play) => ctrl.resume(),
                    Some(SystemEvent::Pause) => ctrl.pause(),
                    Some(SystemEvent::Toggle) => ctrl.toggle(),
                    Some(SystemEvent::Next) => ctrl.play_next(),
                    Some(SystemEvent::Prev) => ctrl.play_prev(),
                    Some(SystemEvent::Seek(secs)) => {
                        ctrl.seek(std::time::Duration::from_secs_f64(secs));
                    }
                    None => {
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                }
            }
        }
    });

    // Android routes media-notification button taps through a JNI callback (no event
    // queue), so we register a background handler like macOS and let the shared loop
    // drain the resulting BgCmds.
    #[cfg(target_os = "android")]
    use_hook(move || {
        init_bg_channel();
        // Runs on the event loop thread, which is the only place its looper can be
        // picked up — see `capture_event_loop`.
        player::systemint::capture_event_loop();
        // Same trick as macOS: the keepalive ticker pokes this while the activity is
        // hidden, so the loop below keeps draining commands and advancing the queue.
        player::systemint::set_tokio_waker(|| {
            if let Some(notify) = BG_NOTIFY.get() {
                notify.notify_one();
            }
        });
        player::systemint::set_background_handler(move |event| {
            use player::systemint::SystemEvent;
            let cmd = match event {
                SystemEvent::Play => BgCmd::Play,
                SystemEvent::Pause => BgCmd::Pause,
                SystemEvent::Toggle => BgCmd::Toggle,
                SystemEvent::Next => BgCmd::Next,
                SystemEvent::Prev => BgCmd::Prev,
                SystemEvent::Stop => BgCmd::Pause,
                SystemEvent::ToggleShuffle => BgCmd::ToggleShuffle,
                SystemEvent::CycleRepeat => BgCmd::CycleRepeat,
            };
            send_bg_cmd(cmd);
        });
    });

    use_effect(move || {
        let host = ctrl.spotify_host.read().clone();
        let access = {
            let cfg = config.read();
            cfg.server
                .as_ref()
                .filter(|s| s.service == MusicService::Spotify)
                .and_then(|s| s.access_token.clone())
                .map(|p| server::spotify::auth::unpack_token(&p).0)
                .filter(|t| !t.is_empty())
        };
        if let (Some(host), Some(access)) = (host, access) {
            spawn(async move {
                host.set_token(access).await;
            });
        }
    });

    use_effect(move || {
        let vol = *ctrl.volume.read();
        if !*ctrl.external_active.read() {
            return;
        }
        if ctrl.spotify_device_override.read().is_some() {
            if let Some(access) = ctrl.spotify_access() {
                let percent = (vol.clamp(0.0, 1.0) * 100.0).round() as u8;
                spawn(async move {
                    let _ = server::spotify::api::player_volume(&access, percent).await;
                });
            }
        } else if let Some(host) = ctrl.spotify_host.read().clone() {
            host.set_volume(vol);
        }
    });

    use_future(move || {
        let mut ctrl = ctrl;
        async move {
            let mut prev_playing = false;
            let mut prev_progress: u64 = 0;
            let mut prev_duration: u64 = 0;
            let mut prev_track_id: Option<String> = None;
            let mut os_artwork: Option<String> = None;
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                if !*ctrl.external_active.peek() || ctrl.spotify_device_override.peek().is_none() {
                    prev_playing = false;
                    prev_progress = 0;
                    prev_track_id = None;
                    os_artwork = None;
                    let discover = !*ctrl.spotify_device_chosen.peek()
                        && ctrl.spotify_device_override.peek().is_none();
                    let access = discover.then(|| ctrl.spotify_access()).flatten();
                    if let Some(access) = access
                        && let Ok(Some(st)) = server::spotify::api::player_state(&access).await
                        && st.is_playing
                        && let Some(dev) = st.device_id
                        && Some(&dev) != ctrl.spotify_device.peek().as_ref()
                    {
                        ctrl.spotify_adopt_external(dev);
                        // Adoption flips `external_active`; reflect the live track
                        // now rather than waiting a poll, so a cold start lands on
                        // what's actually playing instead of the restored last song.
                        if *ctrl.external_active.peek() {
                            let progress = st.progress_ms / 1000;
                            if let Some(track) = st.track.clone() {
                                ctrl.hydrate_external_track_metadata(track, progress);
                            }
                            ctrl.is_playing.set(st.is_playing);
                            ctrl.spotify_progress_anchor
                                .set(Some((st.progress_ms, std::time::Instant::now())));
                            let cover = ctrl.current_song_cover_url.peek().clone();
                            os_artwork = match st.track_id.as_deref() {
                                Some(id) => os_now_playing::cache_artwork(id, &cover).await,
                                None => None,
                            };
                            os_now_playing::push(OsTrack {
                                title: &ctrl.current_song_title.peek(),
                                artist: &ctrl.current_song_artist.peek(),
                                album: &ctrl.current_song_album.peek(),
                                duration_secs: *ctrl.current_song_duration.peek(),
                                position_secs: progress,
                                playing: st.is_playing,
                                artwork: os_artwork.as_deref(),
                            });
                            prev_track_id = st.track_id;
                            prev_playing = st.is_playing;
                            prev_progress = st.progress_ms;
                            prev_duration = st.duration_ms;
                        }
                    }
                    continue;
                }
                let Some(access) = ctrl.spotify_access() else {
                    continue;
                };
                let ended_before =
                    prev_playing && prev_duration > 0 && prev_progress + 5000 >= prev_duration;
                match server::spotify::api::player_state(&access).await {
                    Ok(Some(st)) => {
                        if st.track_id != prev_track_id {
                            if let Some(track) = st.track.clone() {
                                ctrl.hydrate_external_track_metadata(track, st.progress_ms / 1000);
                            }
                            let cover = ctrl.current_song_cover_url.peek().clone();
                            os_artwork = match st.track_id.as_deref() {
                                Some(id) => os_now_playing::cache_artwork(id, &cover).await,
                                None => None,
                            };
                            prev_track_id = st.track_id.clone();
                        }
                        ctrl.is_playing.set(st.is_playing);
                        ctrl.spotify_progress_anchor
                            .set(Some((st.progress_ms, std::time::Instant::now())));
                        ctrl.current_song_progress.set(st.progress_ms / 1000);
                        if st.duration_ms > 0 {
                            ctrl.current_song_duration.set(st.duration_ms / 1000);
                        }
                        os_now_playing::push(OsTrack {
                            title: &ctrl.current_song_title.peek(),
                            artist: &ctrl.current_song_artist.peek(),
                            album: &ctrl.current_song_album.peek(),
                            duration_secs: *ctrl.current_song_duration.peek(),
                            position_secs: st.progress_ms / 1000,
                            playing: st.is_playing,
                            artwork: os_artwork.as_deref(),
                        });
                        let at_end = st.progress_ms == 0
                            || (st.duration_ms > 0 && st.progress_ms + 1500 >= st.duration_ms);
                        if !st.is_playing && ended_before && at_end {
                            prev_playing = false;
                            prev_progress = 0;
                            record_listen(config, &ctrl);
                            ctrl.play_next();
                            continue;
                        }
                        prev_playing = st.is_playing;
                        prev_progress = st.progress_ms;
                        prev_duration = st.duration_ms;
                    }
                    Ok(None) => {
                        if ended_before {
                            record_listen(config, &ctrl);
                            ctrl.play_next();
                        }
                        prev_playing = false;
                        prev_progress = 0;
                        prev_track_id = None;
                        os_artwork = None;
                    }
                    Err(_) => {}
                }
            }
        }
    });

    let mut spotify_pump = use_signal(|| None::<dioxus_core::Task>);
    use_effect(move || {
        let host = ctrl.spotify_host.read().clone();
        if let Some(prev) = spotify_pump.take() {
            prev.cancel();
        }
        let Some(host) = host else { return };
        let mut rx = host.subscribe();
        let mut ctrl = ctrl;
        let mut config = config;
        let task = spawn(async move {
            use server::spotify::host::HostEvent;
            use tokio::sync::broadcast::error::RecvError;
            let mut last_auth_refresh: Option<std::time::Instant> = None;
            let mut loaded_context_uri: Option<String> = None;
            loop {
                let ev = match rx.recv().await {
                    Ok(ev) => ev,
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => break,
                };
                match ev {
                    HostEvent::Ready { device_id } => {
                        ctrl.spotify_device.set(Some(device_id.clone()));
                        if !*ctrl.spotify_activated.peek() {
                            continue;
                        }
                        let pending = ctrl.spotify_pending_uri.peek().clone();
                        if let (Some(uri), Some(access)) = (pending, ctrl.spotify_access()) {
                            ctrl.spotify_pending_uri.set(None);
                            ctrl.start_spotify_uri(access, device_id, uri);
                        }
                    }
                    HostEvent::NotReady => {
                        ctrl.spotify_device.set(None);
                    }
                    HostEvent::BrowserDisconnected => {
                        tracing::info!(
                            "spotify player browser disconnected; waiting for the next play"
                        );
                        ctrl.spotify_browser_disconnected();
                        break;
                    }
                    HostEvent::Media {
                        action,
                        position_ms,
                    } => {
                        if *ctrl.external_active.peek() {
                            match action.as_str() {
                                "play" => ctrl.resume(),
                                "pause" => ctrl.pause(),
                                "next" => ctrl.play_next(),
                                "prev" => ctrl.play_prev(),
                                "seek" => {
                                    if let Some(ms) = position_ms {
                                        ctrl.seek(std::time::Duration::from_millis(ms));
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    HostEvent::State {
                        paused,
                        position_ms,
                        duration_ms,
                        track_id,
                        context_uri,
                        track,
                        ended,
                    } => {
                        if *ctrl.external_active.peek()
                            && ctrl.spotify_device_override.peek().is_none()
                        {
                            if ended {
                                record_listen(config, &ctrl);
                                ctrl.play_next();
                            } else if !ctrl.spotify_report_is_stale(track_id.as_deref()) {
                                ctrl.is_playing.set(!paused);
                                ctrl.spotify_progress_anchor
                                    .set(Some((position_ms, std::time::Instant::now())));
                                ctrl.current_song_progress.set(position_ms / 1000);
                                let shown_id = ctrl
                                    .current_track_snapshot
                                    .peek()
                                    .as_ref()
                                    .map(|shown| shown.id.key().to_string());
                                let track_changed = track_id
                                    .as_deref()
                                    .is_some_and(|id| shown_id.as_deref() != Some(id));
                                if track_changed {
                                    if let Some(track) = track {
                                        ctrl.hydrate_external_track_metadata(
                                            *track,
                                            position_ms / 1000,
                                        );
                                    }
                                    if context_uri.as_ref() != loaded_context_uri.as_ref()
                                        && let (Some(access), Some(context_uri), Some(track_id)) = (
                                            ctrl.spotify_access(),
                                            context_uri.as_deref(),
                                            track_id.as_deref(),
                                        )
                                    {
                                        match server::spotify::api::context_tracks(
                                            &access,
                                            context_uri,
                                        )
                                        .await
                                        {
                                            Ok(Some(tracks)) => {
                                                let still_current = ctrl
                                                    .current_track_snapshot
                                                    .peek()
                                                    .as_ref()
                                                    .is_some_and(|shown| {
                                                        shown.id.key() == track_id
                                                    });
                                                if still_current {
                                                    ctrl.hydrate_external_context(
                                                        tracks,
                                                        track_id,
                                                        position_ms / 1000,
                                                    );
                                                    loaded_context_uri =
                                                        Some(context_uri.to_string());
                                                }
                                            }
                                            Ok(None) => {
                                                loaded_context_uri = Some(context_uri.to_string());
                                            }
                                            Err(e) => tracing::warn!(
                                                error = %e,
                                                context = context_uri,
                                                "spotify playback context could not be loaded"
                                            ),
                                        }
                                    }
                                }
                                if context_uri.is_none() {
                                    loaded_context_uri = None;
                                }
                                if duration_ms > 0 {
                                    ctrl.current_song_duration.set(duration_ms / 1000);
                                }
                            }
                        }
                    }
                    HostEvent::Activated => {
                        ctrl.spotify_activated.set(true);
                        if *ctrl.external_active.peek() {
                            let uri = ctrl.spotify_pending_uri.peek().clone().or_else(|| {
                                ctrl.current_track()
                                    .filter(|t| t.id.service() == Some(MusicService::Spotify))
                                    .map(|t| format!("spotify:track:{}", t.id.key()))
                            });
                            if let (Some(uri), Some(access), Some(device)) = (
                                uri,
                                ctrl.spotify_access(),
                                ctrl.spotify_device.peek().clone(),
                            ) {
                                ctrl.spotify_pending_uri.set(None);
                                let mut error = ctrl.playback_error;
                                let current_device = ctrl.spotify_device;
                                spawn(async move {
                                    if let Err(e) = server::spotify::api::start_playback(
                                        &access,
                                        &device,
                                        &[uri],
                                    )
                                    .await
                                        && current_device.peek().as_deref() == Some(device.as_str())
                                    {
                                        tracing::warn!(error = %e, "spotify activation start failed");
                                        error.set(Some(e));
                                    }
                                });
                            }
                        }
                    }
                    HostEvent::Error { kind, message } => {
                        if kind == "auth"
                            && last_auth_refresh
                                .is_none_or(|t| t.elapsed() > std::time::Duration::from_secs(60))
                        {
                            let creds = {
                                let cfg = config.peek();
                                cfg.server
                                    .as_ref()
                                    .filter(|s| s.service == MusicService::Spotify)
                                    .and_then(|s| {
                                        s.access_token.clone().map(|t| (t, s.url.clone()))
                                    })
                            };
                            if let Some((packed, client_id)) = creds {
                                last_auth_refresh = Some(std::time::Instant::now());
                                let mut error = ctrl.playback_error;
                                spawn(async move {
                                    match server::spotify::auth::refresh_packed(
                                        &packed,
                                        client_id.clone(),
                                    )
                                    .await
                                    {
                                        Ok(new_packed) => {
                                            let mut cfg = config.write();
                                            if let Some(srv) = cfg.server.as_mut()
                                                && srv.service == MusicService::Spotify
                                                && srv.url == client_id
                                                && srv.access_token.as_deref()
                                                    == Some(packed.as_str())
                                            {
                                                srv.access_token = Some(new_packed);
                                                tracing::info!(
                                                    "spotify token refreshed after SDK auth error"
                                                );
                                            }
                                        }
                                        Err(e) => {
                                            tracing::warn!(
                                                error = %e,
                                                "spotify auth-error token refresh failed"
                                            );
                                            error.set(Some(
                                                "Spotify session expired — re-sign in from Settings."
                                                    .to_string(),
                                            ));
                                        }
                                    }
                                });
                                continue;
                            }
                        }
                        let msg = match kind.as_str() {
                            "account" => "Spotify playback needs a Premium account.".to_string(),
                            "auth" => {
                                "Spotify session expired — re-sign in from Settings.".to_string()
                            }
                            "widevine" => "This browser can't play Spotify (no Widevine DRM). \
                                Open the Spotify player tab in Chrome, Edge, or Brave."
                                .to_string(),
                            "license" => message,
                            _ => format!("Spotify player error: {message}"),
                        };
                        ctrl.playback_error.set(Some(msg));
                    }
                }
            }
        });
        spotify_pump.set(Some(task));
    });

    event_pump::use_player_event_pump(ctrl, config);
}
