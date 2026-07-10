#[cfg(target_os = "linux")]
use crate::use_player_controller::LoopMode;
use crate::use_player_controller::PlayerController;
use config::AppConfig;
use config::MusicService;
use dioxus::logger::tracing::Instrument;
use dioxus::prelude::*;
use std::sync::Arc;

use discord_presence::Presence;
use discord_presence::cover_art;

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
}

static BG_CMD_TX: std::sync::OnceLock<std::sync::Mutex<std::sync::mpsc::Sender<BgCmd>>> =
    std::sync::OnceLock::new();
static BG_CMD_RX: std::sync::OnceLock<std::sync::Mutex<std::sync::mpsc::Receiver<BgCmd>>> =
    std::sync::OnceLock::new();
static BG_NOTIFY: std::sync::OnceLock<tokio::sync::Notify> = std::sync::OnceLock::new();

/// Persist a play-count increment as a single-row upsert. The in-memory
/// `config.listen_counts` is bumped by the caller for live views; this is the
/// durable side (no whole-config rewrite on the play hot path).
fn bump_listen_count_db(track_uid: String, db: db::Db) {
    spawn(async move {
        if let Err(e) = ::server::source::local(db)
            .bump_listen_count(&track_uid)
            .await
        {
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
    let presence: Option<Arc<Presence>> = use_context();
    let mut config: Signal<AppConfig> = use_context();

    let mut last_title = use_signal(String::new);
    let mut last_source: Signal<Option<String>> = use_signal(|| None);
    let mut was_playing = use_signal(|| false);
    let mut discord_cover_url: Signal<Option<String>> = use_signal(|| None);
    let mut discord_cover_resolving_for = use_signal(String::new);
    let mut discord_cover_sent = use_signal(|| false);

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
            };
            send_bg_cmd(cmd);
            nudge_event_loop();
        });
    });

    // Keep MPRIS shuffle/repeat in sync with the UI's own toggles.
    #[cfg(target_os = "linux")]
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

        player::systemint::set_background_handler(move |event| {
            use player::systemint::SystemEvent;
            let cmd = match event {
                SystemEvent::Play => BgCmd::Play,
                SystemEvent::Pause => BgCmd::Pause,
                SystemEvent::Toggle => BgCmd::Toggle,
                SystemEvent::Next => BgCmd::Next,
                SystemEvent::Prev => BgCmd::Prev,
                SystemEvent::Stop => BgCmd::Pause,
            };
            send_bg_cmd(cmd);
        });
    });

    let gens = crate::db_reactivity::use_generations();
    use_future(move || {
        let mut ctrl = ctrl;
        let presence = presence.clone();
        let mut last_ping = std::time::Instant::now();
        let mut last_progress_report = std::time::Instant::now();
        let mut last_discord_enabled = false;
        let mut last_jellyfin_id: Option<String> = None;
        #[cfg(target_os = "macos")]
        let mut last_now_playing_refresh = std::time::Instant::now();
        let mut last_lyrics_prefetch_track: Option<String> = None;

        async move {
            let mut last_progress_secs: u64 = u64::MAX;
            let mut prev_playing = false;
            let mut last_recent_path: Option<String> = None;
            let bg_notify = BG_NOTIFY.get_or_init(tokio::sync::Notify::new);
            // Engine events (Position once per second while playing, Ended,
            // phase changes) wake this loop; the coarse timer only covers the
            // genuinely periodic work (Jellyfin reports, Discord, refreshes).
            let mut engine_events = ctrl.player.peek().subscribe();
            loop {
                let mut woke_event = None;
                tokio::select! {
                    _ = bg_notify.notified() => {},
                    _ = tokio::time::sleep(std::time::Duration::from_millis(1000)) => {},
                    event = engine_events.recv() => { woke_event = event; },
                }
                // Drain whatever queued while we were asleep. Progress and
                // completion are read from the live status snapshot below;
                // only phase transitions need the events themselves.
                while let Some(event) = woke_event.take().or_else(|| engine_events.try_recv().ok())
                {
                    use player::engine::{Event, Phase};
                    match event {
                        // Playing/Paused confirm audible state for a session. A
                        // phase for a token we no longer intend is stale (a
                        // superseded/cancelled load the engine still promoted);
                        // if it started playing, stop it — the UI intent is
                        // authoritative, we never adopt a navigated-away track.
                        Event::PhaseChanged {
                            token,
                            phase: phase @ (Phase::Playing | Phase::Paused),
                        } => {
                            if token == ctrl.intent.peek().token() {
                                ctrl.is_playing.set(phase == Phase::Playing);
                            } else if phase == Phase::Playing {
                                ctrl.player.peek().stop_for_transition();
                            }
                        }
                        // Idle means the engine holds no session — nothing is
                        // audible (a full stop, or a device loss that dropped a
                        // non-seekable radio stream). Ended is deliberately left
                        // to the auto-advance check below, which needs is_playing.
                        Event::PhaseChanged {
                            phase: Phase::Idle, ..
                        } => ctrl.is_playing.set(false),
                        // A crossfade finished: commit the deferred UI to the
                        // incoming track exactly when the engine tore the
                        // outgoing session down, not on a wall-clock guess.
                        Event::TrackSwitched { token, .. } => {
                            ctrl.commit_transition(token);
                            last_progress_secs = u64::MAX;
                        }
                        // Loaded confirms a session started; if it's for a load
                        // we superseded or cancelled (end-of-queue racing a
                        // promoted load), stop it instead of adopting it.
                        Event::Loaded { token } if token != ctrl.intent.peek().token() => {
                            ctrl.player.peek().stop_for_transition();
                        }
                        // A device lost mid-playback on a live session (radio
                        // unplug): the load reply can't report this, so surface
                        // the banner and stop here. No auto-reconnect — Play
                        // re-loads the station.
                        Event::Error { token, message } if token == ctrl.intent.peek().token() => {
                            tracing::warn!(%message, "engine reported a playback error");
                            ctrl.fail_load(token, &message);
                            ctrl.is_playing.set(false);
                        }
                        _ => {}
                    }
                }

                nudge_event_loop();

                for cmd in drain_bg_cmds() {
                    match cmd {
                        BgCmd::Play => ctrl.resume(),
                        BgCmd::Pause => ctrl.pause(),
                        BgCmd::Toggle => ctrl.toggle(),
                        BgCmd::Next => ctrl.play_next(),
                        BgCmd::Prev => ctrl.play_prev(),
                    }
                }

                let is_playing = *ctrl.is_playing.read();

                {
                    let current_track = {
                        let idx = *ctrl.current_queue_index.read();
                        ctrl.get_track_at(idx)
                    };
                    if let Some(track) = current_track
                        && is_playing
                    {
                        let uid = track.id.uid().to_string();
                        if last_recent_path.as_ref() != Some(&uid) {
                            last_recent_path = Some(uid);
                            // Records under the active source's partition — same
                            // source the rest of now-playing resolves through.
                            let key = track.id.key().into_owned();
                            let source = ctrl.active_source.peek().clone();
                            spawn(async move {
                                if source.record_recent(&key).await.is_ok() {
                                    gens.bump(crate::db_reactivity::Table::Recents);
                                }
                            });
                        }
                    }
                }

                let lyrics_prefetch = {
                    let current_idx = *ctrl.current_queue_index.read();
                    ctrl.get_track_at(current_idx + 1)
                };

                if let Some(next_track) = lyrics_prefetch {
                    let next_track_key = next_track.id.uid().to_string();
                    if last_lyrics_prefetch_track.as_ref() != Some(&next_track_key) {
                        last_lyrics_prefetch_track = Some(next_track_key);
                        let (
                            server_url,
                            server_token,
                            server_user_id,
                            prefer_local,
                            enable_musixmatch,
                        ) = {
                            let conf = config.read();
                            let prefer_local = conf.prefer_local_lyrics;
                            let enable_musixmatch = conf.enable_musixmatch_lyrics;
                            if let Some(server) = &conf.server {
                                (
                                    Some(server.url.clone()),
                                    server.access_token.clone(),
                                    server.user_id.clone(),
                                    prefer_local,
                                    enable_musixmatch,
                                )
                            } else {
                                (None, None, None, prefer_local, enable_musixmatch)
                            }
                        };

                        spawn(async move {
                            let next_track_path = next_track.id.uid();
                            let lyrics_request = utils::lyrics::LyricsRequest::new(
                                next_track.artist,
                                next_track.title,
                                next_track.album,
                                next_track.duration,
                                next_track_path,
                            )
                            .with_server(
                                server_url.as_deref(),
                                server_token.as_deref(),
                                server_user_id.as_deref(),
                            )
                            .prefer_local(prefer_local)
                            .enable_musixmatch(enable_musixmatch);
                            let _ = utils::lyrics::fetch_lyrics_for_request(&lyrics_request).await;
                        });
                    }
                }

                // Android has no Discord; force-disable so the cover-art resolution and
                // presence updates below are all skipped (the Presence context is None too).
                #[cfg(target_os = "android")]
                let discord_enabled = false;
                #[cfg(not(target_os = "android"))]
                let discord_enabled = config.read().discord_presence.unwrap_or(true);
                let discord_paused_enabled = config.read().discord_presence_paused.unwrap_or(true);
                let discord_source_name = config
                    .read()
                    .discord_presence_source
                    .unwrap_or(true)
                    .then(|| {
                        config
                            .read()
                            .active_service()
                            .map_or("Local", |s| s.display_name())
                    });
                let source_changed = last_source.peek().as_deref() != discord_source_name;

                // Discord may start after Kopuz or restart mid-session: retry a
                // dropped IPC connection and restore the last activity once it's up.
                if discord_enabled && let Some(ref p) = presence {
                    p.tick();
                }

                let pos = ctrl.player.read().get_position();
                let mut defer_player_progress = false;

                // While a crossfade's UI is deferred, show the outgoing (fading)
                // track's live position from the engine and hold back the
                // incoming session's; the actual commit happens on the engine's
                // TrackSwitched event, not a wall-clock estimate.
                if ctrl.pending_crossfade_ui.peek().is_some() {
                    if let Some(fading) = ctrl.player.read().fading_position() {
                        ctrl.current_song_progress.set(fading.as_secs());
                    }
                    defer_player_progress = true;
                }

                let jellyfin_info = {
                    let conf = config.read();
                    conf.server.clone().map(|s| (s, conf.device_id.clone()))
                };

                if let Some((server, _device_id)) = jellyfin_info
                    && server.service == MusicService::Jellyfin
                {
                    // Session reporting goes through the active source (Jellyfin
                    // overrides keepalive/report_*; others no-op). Reports are
                    // gated to ≥5s/track-change, so resolving a fresh source per
                    // report is cheap — no client cache needed.
                    if last_ping.elapsed().as_secs() >= 30 {
                        let source = ctrl.active_source.peek().clone();
                        spawn(
                            async move {
                                let _ = source.keepalive().await;
                            }
                            .instrument(tracing::info_span!("jellyfin.keepalive")),
                        );
                        last_ping = std::time::Instant::now();
                    }

                    let track = {
                        let current_idx = *ctrl.current_queue_index.read();
                        ctrl.get_track_at(current_idx)
                    };

                    if let Some(track) = track {
                        if let Some(current_id) = track
                            .id
                            .service()
                            .filter(|s| *s == MusicService::Jellyfin)
                            .map(|_| track.id.key().into_owned())
                        {
                            if last_jellyfin_id.as_ref() != Some(&current_id) {
                                if let Some(old_id) = last_jellyfin_id {
                                    let source = ctrl.active_source.peek().clone();
                                    let ticks = pos.as_micros() as u64 * 10;
                                    spawn(
                                        async move {
                                            let _ = source
                                                .report_playback_stopped(&old_id, ticks)
                                                .await;
                                        }
                                        .instrument(tracing::info_span!("playback.report")),
                                    );
                                }
                                let source = ctrl.active_source.peek().clone();
                                let current_id_clone = current_id.clone();
                                spawn(
                                    async move {
                                        let _ =
                                            source.report_playback_start(&current_id_clone).await;
                                    }
                                    .instrument(tracing::info_span!("playback.report")),
                                );
                                last_jellyfin_id = Some(current_id.clone());
                            }

                            if last_progress_report.elapsed().as_secs() >= 5
                                || is_playing != prev_playing
                            {
                                let ticks = pos.as_micros() as u64 * 10;
                                let source = ctrl.active_source.peek().clone();
                                let current_id_clone = current_id.clone();
                                spawn(
                                    async move {
                                        let _ = source
                                            .report_playback_progress(
                                                &current_id_clone,
                                                ticks,
                                                !is_playing,
                                            )
                                            .await;
                                    }
                                    .instrument(tracing::info_span!("playback.report")),
                                );
                                last_progress_report = std::time::Instant::now();
                            }
                        } else if let Some(old_id) = last_jellyfin_id.take() {
                            let source = ctrl.active_source.peek().clone();
                            let ticks = pos.as_micros() as u64 * 10;
                            spawn(
                                async move {
                                    let _ = source.report_playback_stopped(&old_id, ticks).await;
                                }
                                .instrument(tracing::info_span!("playback.report")),
                            );
                        }
                    } else if let Some(old_id) = last_jellyfin_id.take() {
                        let source = ctrl.active_source.peek().clone();
                        let ticks = pos.as_micros() as u64 * 10;
                        spawn(
                            async move {
                                let _ = source.report_playback_stopped(&old_id, ticks).await;
                            }
                            .instrument(tracing::info_span!("playback.report")),
                        );
                    }
                }

                #[cfg(target_os = "macos")]
                if last_now_playing_refresh.elapsed().as_secs() >= 10 {
                    player::systemint::refresh_now_playing();
                    last_now_playing_refresh = std::time::Instant::now();
                }

                if is_playing {
                    let duration = *ctrl.current_song_duration.read();
                    let pos_secs = pos.as_secs().min(duration);
                    let current_token = *ctrl.current_token.read();
                    if !defer_player_progress && pos_secs != last_progress_secs {
                        last_progress_secs = pos_secs;
                        ctrl.current_song_progress.set(pos_secs);
                    }

                    if let Some(ref p) = presence {
                        let title = ctrl.current_song_title.read().clone();
                        let artist = ctrl.current_song_artist.read().clone();
                        let album = ctrl.current_song_album.read().clone();
                        let duration = *ctrl.current_song_duration.read();
                        let progress = if duration == u64::MAX {
                            0
                        } else {
                            pos.as_secs()
                        };

                        let song_key = format!("{}|{}|{}", title, artist, album);

                        if discord_enabled && song_key != *discord_cover_resolving_for.peek() {
                            discord_cover_resolving_for.set(song_key.clone());
                            discord_cover_url.set(None);
                            discord_cover_sent.set(false);

                            let mbid = {
                                let idx = *ctrl.current_queue_index.read();
                                ctrl.get_track_at(idx)
                                    .and_then(|t| t.musicbrainz_release_id.clone())
                            };
                            let artist_c = artist.clone();
                            let album_c = album.clone();
                            let song_key_for_spawn = song_key.clone();
                            spawn(
                                async move {
                                    let resolved = cover_art::resolve_cover_art_url_cached(
                                        mbid.as_deref(),
                                        &artist_c,
                                        &album_c,
                                    )
                                    .await;
                                    if *discord_cover_resolving_for.peek() == song_key_for_spawn {
                                        discord_cover_url.set(resolved);
                                    }
                                }
                                .instrument(tracing::info_span!("presence.cover_resolve")),
                            );
                        }

                        if discord_enabled {
                            let song_changed = title != *last_title.peek();
                            let resumed = !*was_playing.peek();
                            let toggled_on = !last_discord_enabled;
                            let cover_just_resolved =
                                discord_cover_url.peek().is_some() && !*discord_cover_sent.peek();

                            if song_changed
                                || resumed
                                || toggled_on
                                || cover_just_resolved
                                || source_changed
                            {
                                last_title.set(title.clone());
                                last_source.set(discord_source_name.map(str::to_owned));

                                let resolved = discord_cover_url.read().clone();
                                let cover_ref = resolved.as_deref();

                                let _ = p.set_now_playing(
                                    &title,
                                    &artist,
                                    &album,
                                    progress,
                                    duration,
                                    cover_ref,
                                    discord_source_name,
                                );

                                if resolved.is_some() {
                                    discord_cover_sent.set(true);
                                }
                            }
                        } else if last_discord_enabled {
                            let _ = p.clear_activity();
                        }
                    }

                    // A transition is in flight from when a crossfade arms
                    // (is_loading, while the next stream resolves) until its
                    // deferred UI commits (pending_crossfade_ui). Arming or
                    // skipping during that window double-fires: `pos` is the
                    // incoming track's position while `duration` is still the
                    // outgoing track's, so `remaining` is nonsense, and the
                    // engine may still be finishing the outgoing track near its
                    // end — which re-satisfies the arm and stacks transitions.
                    let transition_in_flight =
                        *ctrl.is_loading.read() || ctrl.pending_crossfade_ui.read().is_some();

                    let remaining_secs = duration.saturating_sub(pos_secs);
                    let should_crossfade = duration > 0
                        && pos_secs < duration
                        && ctrl.should_crossfade()
                        && ctrl.has_next_track()
                        && remaining_secs <= config.read().crossfade_seconds as u64
                        && *ctrl.armed_transition.peek() != Some(current_token);

                    if should_crossfade && !transition_in_flight {
                        ctrl.armed_transition.set(Some(current_token));
                        {
                            let mut config_write = config.write();
                            let idx = *ctrl.current_queue_index.peek();
                            if let Some(track) = ctrl.get_track_at(idx) {
                                let track_id = track.id.uid().to_string();
                                *config_write
                                    .listen_counts
                                    .entry(track_id.clone())
                                    .or_insert(0) += 1;
                                drop(config_write);
                                bump_listen_count_db(track_id, ctrl.db.peek().clone());
                            }
                        }
                        ctrl.play_next_with_crossfade();
                        nudge_event_loop();
                        prev_playing = is_playing;
                        {
                            was_playing.set(is_playing);
                            last_discord_enabled = discord_enabled;
                        }
                        continue;
                    }

                    let is_radio = duration == u64::MAX;
                    let should_skip = if is_radio {
                        false
                    } else {
                        ctrl.player.read().is_playback_complete()
                            || (duration > 0 && pos.as_secs() >= duration.saturating_add(5))
                    };

                    if should_skip && !transition_in_flight {
                        if !is_radio && duration > 0 && last_progress_secs != duration {
                            last_progress_secs = duration;
                            ctrl.current_song_progress.set(duration);
                        }
                        {
                            let mut config_write = config.write();
                            let _q = ctrl.queue.peek();
                            let idx = *ctrl.current_queue_index.peek();
                            if let Some(track) = ctrl.get_track_at(idx) {
                                let track_id = track.id.uid().to_string();
                                *config_write
                                    .listen_counts
                                    .entry(track_id.clone())
                                    .or_insert(0) += 1;
                                drop(config_write);
                                bump_listen_count_db(track_id, ctrl.db.peek().clone());
                            }
                        }
                        ctrl.play_next();
                        nudge_event_loop();
                    }
                } else {
                    if *was_playing.peek() {
                        if let Some(ref p) = presence {
                            let title = ctrl.current_song_title.read().clone();
                            let artist = ctrl.current_song_artist.read().clone();
                            let album = ctrl.current_song_album.read().clone();
                            if discord_enabled && discord_paused_enabled {
                                let resolved = discord_cover_url.read().clone();
                                last_source.set(discord_source_name.map(str::to_owned));
                                let _ = p.set_paused(
                                    &title,
                                    &artist,
                                    &album,
                                    resolved.as_deref(),
                                    discord_source_name,
                                );
                            } else if last_discord_enabled || !discord_paused_enabled {
                                let _ = p.clear_activity();
                            }
                        }
                    } else if let Some(ref p) = presence {
                        if !discord_enabled && last_discord_enabled {
                            let _ = p.clear_activity();
                        } else if discord_enabled
                            && (!last_discord_enabled || (source_changed && discord_paused_enabled))
                        {
                            let title = ctrl.current_song_title.read().clone();
                            if !title.is_empty() {
                                let artist = ctrl.current_song_artist.read().clone();
                                let album = ctrl.current_song_album.read().clone();
                                let resolved = discord_cover_url.read().clone();
                                last_source.set(discord_source_name.map(str::to_owned));
                                let _ = p.set_paused(
                                    &title,
                                    &artist,
                                    &album,
                                    resolved.as_deref(),
                                    discord_source_name,
                                );
                            }
                        }
                    }
                }

                prev_playing = is_playing;
                {
                    was_playing.set(is_playing);
                    last_discord_enabled = discord_enabled;
                }
            }
        }
    });
}
