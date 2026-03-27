use crate::use_player_controller::PlayerController;
use config::AppConfig;
use dioxus::prelude::*;
use discord_presence::Presence;
use discord_presence::cover_art;
use server::jellyfin::JellyfinRemote;
use std::sync::Arc;

#[cfg(target_os = "macos")]
use player::systemint::set_background_handler;

#[cfg(target_os = "macos")]
use player::systemint::set_tokio_waker;

#[derive(Debug, Clone, Copy)]
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

fn init_bg_channel() {
    BG_CMD_TX.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<BgCmd>();
        let _ = BG_CMD_RX.set(std::sync::Mutex::new(rx));
        std::sync::Mutex::new(tx)
    });
    BG_NOTIFY.get_or_init(tokio::sync::Notify::new);
}

fn send_bg_cmd(cmd: BgCmd) {
    if let Some(lock) = BG_CMD_TX.get() {
        if let Ok(tx) = lock.lock() {
            let _ = tx.send(cmd);
        }
    }
    // Instantly wake the tokio task so it processes the command
    // without waiting for the next 250ms poll tick.
    if let Some(notify) = BG_NOTIFY.get() {
        notify.notify_one();
    }
}

fn drain_bg_cmds() -> Vec<BgCmd> {
    let mut cmds = Vec::new();
    if let Some(lock) = BG_CMD_RX.get() {
        if let Ok(rx) = lock.try_lock() {
            while let Ok(cmd) = rx.try_recv() {
                cmds.push(cmd);
            }
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

    #[cfg(target_os = "linux")]
    use_future(move || {
        let mut ctrl = ctrl;
        async move {
            use player::systemint::{SystemEvent, poll_event};
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
            println!("[player_task] Starting Windows SMTC event loop");
            loop {
                match wait_event().await {
                    Some(SystemEvent::Play) => ctrl.resume(),
                    Some(SystemEvent::Pause) => ctrl.pause(),
                    Some(SystemEvent::Toggle) => ctrl.toggle(),
                    Some(SystemEvent::Next) => ctrl.play_next(),
                    Some(SystemEvent::Prev) => ctrl.play_prev(),
                    Some(SystemEvent::Seek(secs)) => {
                        ctrl.player
                            .write()
                            .seek(std::time::Duration::from_secs_f64(secs));
                    }
                    None => {
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                }
            }
        }
    });

    use_future(move || {
        let mut ctrl = ctrl;
        let presence = presence.clone();
        let mut last_discord_enabled = false;
        let mut last_jellyfin_id: Option<String> = None;
        let mut last_ping = std::time::Instant::now();
        let mut last_progress_report = std::time::Instant::now();
        #[cfg(target_os = "macos")]
        let mut last_now_playing_refresh = std::time::Instant::now();

        async move {
            let mut last_progress_secs: u64 = u64::MAX;
            let bg_notify = BG_NOTIFY.get_or_init(tokio::sync::Notify::new);
            loop {
                // Wait for EITHER a media command notification OR 250ms,
                // whichever comes first. This ensures instant response to
                // media keys even when macOS is coalescing tokio timers.
                tokio::select! {
                    _ = bg_notify.notified() => {},
                    _ = tokio::time::sleep(std::time::Duration::from_millis(250)) => {},
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
                let discord_enabled = config.read().discord_presence.unwrap_or(true);
                let pos = ctrl.player.read().get_position();

                let jellyfin_info = {
                    let conf = config.read();
                    conf.server.clone().map(|s| (s, conf.device_id.clone()))
                };

                if let Some((server, device_id)) = jellyfin_info {
                    let remote = Arc::new(JellyfinRemote::new(
                        &server.url,
                        server.access_token.as_deref(),
                        &device_id,
                        server.user_id.as_deref(),
                    ));

                    if last_ping.elapsed().as_secs() >= 30 {
                        let remote = remote.clone();
                        spawn(async move {
                            let _ = remote.ping().await;
                        });
                        last_ping = std::time::Instant::now();
                    }

                    let track = {
                        let q = ctrl.queue.read();
                        let idx = *ctrl.current_queue_index.read();
                        q.get(idx).cloned()
                    };

                    if let Some(track) = track {
                        let path_str = track.path.to_string_lossy();
                        if path_str.starts_with("jellyfin:") {
                            let parts: Vec<&str> = path_str.split(':').collect();
                            if let Some(id) = parts.get(1) {
                                let current_id = id.to_string();

                                if last_jellyfin_id.as_ref() != Some(&current_id) {
                                    if let Some(old_id) = last_jellyfin_id {
                                        let remote = remote.clone();
                                        spawn(async move {
                                            let _ = remote
                                                .report_playback_stopped(
                                                    &old_id,
                                                    pos.as_micros() as u64 * 10,
                                                )
                                                .await;
                                        });
                                    }
                                    let remote = remote.clone();
                                    let current_id_clone = current_id.clone();
                                    spawn(async move {
                                        let _ =
                                            remote.report_playback_start(&current_id_clone).await;
                                    });
                                    last_jellyfin_id = Some(current_id.clone());
                                }

                                if last_progress_report.elapsed().as_secs() >= 5
                                    || is_playing != *was_playing.peek()
                                {
                                    let ticks = pos.as_micros() as u64 * 10;
                                    let remote = remote.clone();
                                    let current_id_clone = current_id.clone();
                                    spawn(async move {
                                        let _ = remote
                                            .report_playback_progress(
                                                &current_id_clone,
                                                ticks,
                                                !is_playing,
                                            )
                                            .await;
                                    });
                                    last_progress_report = std::time::Instant::now();
                                }
                            }
                        } else if let Some(old_id) = last_jellyfin_id.take() {
                            let remote = remote.clone();
                            spawn(async move {
                                let _ = remote
                                    .report_playback_stopped(&old_id, pos.as_micros() as u64 * 10)
                                    .await;
                            });
                        }
                    } else if let Some(old_id) = last_jellyfin_id.take() {
                        let remote = remote.clone();
                        spawn(async move {
                            let _ = remote
                                .report_playback_stopped(&old_id, pos.as_micros() as u64 * 10)
                                .await;
                        });
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
                    if pos_secs != last_progress_secs {
                        last_progress_secs = pos_secs;
                        ctrl.current_song_progress.set(pos_secs);
                    }

                    if let Some(ref p) = presence {
                        let title = ctrl.current_song_title.read().clone();
                        let artist = ctrl.current_song_artist.read().clone();
                        let album = ctrl.current_song_album.read().clone();
                        let duration = *ctrl.current_song_duration.read();
                        let progress = pos.as_secs();
                        let cover = ctrl.current_song_cover_url.read().clone();

                        let song_key = format!("{}|{}|{}", title, artist, album);

                        if discord_enabled && song_key != *discord_cover_resolving_for.peek() {
                            discord_cover_resolving_for.set(song_key);
                            discord_cover_url.set(None);
                            discord_cover_sent.set(false);

                            if cover.starts_with("http") {
                                discord_cover_url.set(Some(cover.clone()));
                            } else {
                                let mbid = {
                                    let q = ctrl.queue.read();
                                    let idx = *ctrl.current_queue_index.read();
                                    q.get(idx).and_then(|t| t.musicbrainz_release_id.clone())
                                };
                                let artist_c = artist.clone();
                                let album_c = album.clone();
                                spawn(async move {
                                    let resolved = cover_art::resolve_cover_art_url(
                                        mbid.as_deref(),
                                        &artist_c,
                                        &album_c,
                                    )
                                    .await;
                                    discord_cover_url.set(resolved);
                                });
                            }
                        }

                        if discord_enabled {
                            let song_changed = title != *last_title.peek();
                            let resumed = !*was_playing.peek();
                            let toggled_on = !last_discord_enabled;
                            let cover_just_resolved =
                                discord_cover_url.peek().is_some() && !*discord_cover_sent.peek();

                            if song_changed || resumed || toggled_on || cover_just_resolved {
                                last_title.set(title.clone());

                                let resolved = discord_cover_url.read().clone();
                                let cover_ref = if let Some(ref url) = resolved {
                                    Some(url.as_str())
                                } else if cover.starts_with("http") {
                                    Some(cover.as_str())
                                } else {
                                    None
                                };

                                let _ = p.set_now_playing(
                                    &title, &artist, &album, progress, duration, cover_ref,
                                );

                                if resolved.is_some() {
                                    discord_cover_sent.set(true);
                                }
                            }
                        } else if last_discord_enabled {
                            let _ = p.clear_activity();
                        }
                    }

                    let is_jellyfin = {
                        let q = ctrl.queue.read();
                        let idx = *ctrl.current_queue_index.read();
                        q.get(idx)
                            .map(|t| t.path.to_string_lossy().starts_with("jellyfin:"))
                            .unwrap_or(false)
                    };

                    let should_skip = if is_jellyfin {
                        duration > 0 && pos.as_secs() >= duration
                    } else {
                        ctrl.player.read().is_empty() || (duration > 0 && pos.as_secs() >= duration)
                    };

                    if should_skip && !*ctrl.is_loading.read() && !*ctrl.skip_in_progress.read() {
                        ctrl.skip_in_progress.set(true);
                        {
                            let mut config_write = config.write();
                            let q = ctrl.queue.peek();
                            let idx = *ctrl.current_queue_index.peek();
                            if let Some(track) = q.get(idx) {
                                let track_id = track.path.to_string_lossy().to_string();
                                *config_write.listen_counts.entry(track_id).or_insert(0) += 1;
                            }
                        }
                        ctrl.play_next();
                        nudge_event_loop();
                    }
                } else if *was_playing.peek() {
                    if let Some(ref p) = presence {
                        let title = ctrl.current_song_title.read().clone();
                        let artist = ctrl.current_song_artist.read().clone();
                        let album = ctrl.current_song_album.read().clone();
                        if discord_enabled {
                            let resolved = discord_cover_url.read().clone();
                            let _ = p.set_paused(&title, &artist, &album, resolved.as_deref());
                        } else if last_discord_enabled {
                            let _ = p.clear_activity();
                        }
                    }
                } else if let Some(ref p) = presence {
                    if !discord_enabled && last_discord_enabled {
                        let _ = p.clear_activity();
                    } else if discord_enabled && !last_discord_enabled {
                        let title = ctrl.current_song_title.read().clone();
                        if !title.is_empty() {
                            let artist = ctrl.current_song_artist.read().clone();
                            let album = ctrl.current_song_album.read().clone();
                            let resolved = discord_cover_url.read().clone();
                            let _ = p.set_paused(&title, &artist, &album, resolved.as_deref());
                        }
                    }
                }

                was_playing.set(is_playing);
                last_discord_enabled = discord_enabled;
            }
        }
    });
}
