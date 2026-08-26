//! Frontend-side async playback orchestration. Engine playback and OS media
//! integration live in the daemon now; what remains here is the Spotify
//! external-playback machinery, which is tied to the browser host and the
//! controller's signal mirror.

#[cfg(any(target_os = "linux", target_os = "android"))]
use crate::use_player_controller::LoopMode;
use crate::use_player_controller::PlayerController;
use config::MusicService;
use dioxus::prelude::*;

#[cfg(target_os = "android")]
mod android_media {
    #[derive(Debug, Clone, Copy)]
    pub(super) enum BgCmd {
        Play,
        Pause,
        Toggle,
        Next,
        Prev,
        ToggleShuffle,
        CycleRepeat,
    }

    pub(super) static BG_CMD_TX: std::sync::OnceLock<
        std::sync::Mutex<std::sync::mpsc::Sender<BgCmd>>,
    > = std::sync::OnceLock::new();
    pub(super) static BG_CMD_RX: std::sync::OnceLock<
        std::sync::Mutex<std::sync::mpsc::Receiver<BgCmd>>,
    > = std::sync::OnceLock::new();
    pub(super) static BG_NOTIFY: std::sync::OnceLock<tokio::sync::Notify> =
        std::sync::OnceLock::new();

    pub(super) fn init_bg_channel() {
        BG_CMD_TX.get_or_init(|| {
            let (tx, rx) = std::sync::mpsc::channel::<BgCmd>();
            let _ = BG_CMD_RX.set(std::sync::Mutex::new(rx));
            std::sync::Mutex::new(tx)
        });
        BG_NOTIFY.get_or_init(tokio::sync::Notify::new);
    }

    pub(super) fn send_bg_cmd(cmd: BgCmd) {
        if let Some(lock) = BG_CMD_TX.get()
            && let Ok(tx) = lock.lock()
        {
            let _ = tx.send(cmd);
        }
        if let Some(notify) = BG_NOTIFY.get() {
            notify.notify_one();
        }
    }

    pub(super) fn drain_bg_cmds() -> Vec<BgCmd> {
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
}

fn report_completed(ctrl: &PlayerController) {
    let track = ctrl.current_track();
    let position_ms = ctrl.current_song_progress.peek().saturating_mul(1000);
    let device = ctrl.spotify_device_override.peek().clone();
    ctrl.session
        .peek()
        .report_external_detached(track, position_ms, false, true, device);
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

/// Platforms whose OS media widget Kopuz drives directly. Engine playback
/// feeds these from the daemon's os_media task; for remote Spotify playback
/// the engine is stopped, so the poll loop here feeds them itself.
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

pub fn use_player_task(mut ctrl: PlayerController) {
    let api = use_context::<std::sync::Arc<dyn api::KopuzApi>>();
    let spotify_access_api = api.clone();

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

    // Android routes media-notification button taps through a JNI callback (no
    // event queue) and the daemon's os_media task has no Android backend, so
    // the taps are drained here and dispatched through the controller.
    #[cfg(target_os = "android")]
    {
        use android_media::BgCmd;
        use_hook(move || {
            android_media::init_bg_channel();
            // Runs on the event loop thread, which is the only place its looper
            // can be picked up — see `capture_event_loop`.
            player::systemint::capture_event_loop();
            // The keepalive ticker pokes this while the activity is hidden, so
            // the loop below keeps draining commands and advancing the queue.
            player::systemint::set_tokio_waker(|| {
                if let Some(notify) = android_media::BG_NOTIFY.get() {
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
                android_media::send_bg_cmd(cmd);
            });
        });
        use_future(move || {
            let mut ctrl = ctrl;
            async move {
                loop {
                    for cmd in android_media::drain_bg_cmds() {
                        match cmd {
                            BgCmd::Play => ctrl.resume(),
                            BgCmd::Pause => ctrl.pause(),
                            BgCmd::Toggle => ctrl.toggle(),
                            BgCmd::Next => ctrl.play_next(),
                            BgCmd::Prev => ctrl.play_prev(),
                            BgCmd::ToggleShuffle => ctrl.toggle_shuffle(),
                            BgCmd::CycleRepeat => ctrl.toggle_loop(),
                        }
                    }
                    let notified = async {
                        if let Some(notify) = android_media::BG_NOTIFY.get() {
                            notify.notified().await;
                        } else {
                            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                        }
                    };
                    tokio::select! {
                        _ = notified => {}
                        _ = tokio::time::sleep(std::time::Duration::from_millis(250)) => {}
                    }
                }
            }
        });
    }

    let sources = use_context::<Signal<Vec<api::SourceInfo>>>();
    let spotify_identity = use_memo(move || {
        sources
            .read()
            .iter()
            .find(|source| source.active && source.service == Some(api::MusicService::Spotify))
            .map(|source| source.id.clone())
    });
    use_effect(move || {
        let host = ctrl.spotify_host.read().clone();
        if spotify_identity.read().is_none() {
            ctrl.spotify_token.set(None);
            return;
        }
        let api = spotify_access_api.clone();
        let mut token = ctrl.spotify_token;
        spawn(async move {
            match api.external_access("spotify".to_string()).await {
                Ok(access) => {
                    token.set(Some(access.access_token.clone()));
                    if let Some(host) = host {
                        host.set_token(access.access_token).await;
                    }
                }
                Err(error) => {
                    token.set(None);
                    tracing::warn!(%error, "could not acquire Spotify frontend access");
                }
            }
        });
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
                            report_completed(&ctrl);
                            ctrl.play_next();
                            continue;
                        }
                        prev_playing = st.is_playing;
                        prev_progress = st.progress_ms;
                        prev_duration = st.duration_ms;
                    }
                    Ok(None) => {
                        if ended_before {
                            report_completed(&ctrl);
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
    let spotify_api = api.clone();
    use_effect(move || {
        let host = ctrl.spotify_host.read().clone();
        if let Some(prev) = spotify_pump.take() {
            prev.cancel();
        }
        let Some(host) = host else { return };
        let mut rx = host.subscribe();
        let mut ctrl = ctrl;
        let api = spotify_api.clone();
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
                                report_completed(&ctrl);
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
                            last_auth_refresh = Some(std::time::Instant::now());
                            match api.external_access("spotify".to_string()).await {
                                Ok(access) => {
                                    ctrl.spotify_token.set(Some(access.access_token.clone()));
                                    let spotify_host = ctrl.spotify_host.peek().clone();
                                    if let Some(host) = spotify_host {
                                        host.set_token(access.access_token).await;
                                    }
                                    tracing::info!("spotify token refreshed after SDK auth error");
                                    continue;
                                }
                                Err(error) => tracing::warn!(
                                    %error,
                                    "spotify auth-error token refresh failed"
                                ),
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
}
