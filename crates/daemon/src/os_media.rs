//! OS media-key integration for the daemon process: MPRIS (Linux), SMTC
//! (Windows), and MPRemoteCommandCenter (macOS) commands flow into the
//! session; shuffle/repeat modes flow back out where the platform shows them.
//!
//! Now-playing metadata itself is pushed by the player engine
//! (`Player::push_now_playing`), so this module only handles the command
//! direction plus mode mirroring. On macOS the callbacks require the process
//! main thread to run a CFRunLoop; the kopuzd binary parks its main thread
//! there via `player::systemint::park_main_loop`.

#![cfg_attr(target_os = "android", allow(unused))]

#[cfg(target_os = "linux")]
use api::LoopMode;
use api::PlayerCommand;

use crate::session::SessionHandle;

async fn command(session: &SessionHandle, command: PlayerCommand) {
    if let Err(error) = session.player_command(command).await {
        tracing::debug!(%error, "media key command rejected");
    }
}

fn seek_command(seconds: f64) -> PlayerCommand {
    PlayerCommand::Seek {
        position_ms: (seconds.max(0.0) * 1000.0) as u64,
    }
}

#[cfg(target_os = "macos")]
pub fn spawn(session: &SessionHandle) {
    use player::systemint::SystemEvent;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<SystemEvent>();
    player::systemint::set_background_handler(move |event| {
        let _ = tx.send(event);
    });
    let session = session.clone();
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            let mapped = match event {
                SystemEvent::Play => PlayerCommand::Play,
                SystemEvent::Pause => PlayerCommand::Pause,
                SystemEvent::Toggle => PlayerCommand::Toggle,
                SystemEvent::Next => PlayerCommand::Next,
                SystemEvent::Prev => PlayerCommand::Previous,
                SystemEvent::Seek(seconds) => seek_command(seconds),
            };
            command(&session, mapped).await;
        }
    });
}

#[cfg(target_os = "linux")]
pub fn spawn(session: &SessionHandle) {
    use api::ApiEvent;
    use player::systemint::{RepeatMode, SystemEvent};

    let session = session.clone();
    let mut events = session.subscribe();
    tokio::spawn(async move {
        let mut last_modes: Option<(bool, LoopMode)> = None;
        loop {
            tokio::select! {
                media = player::systemint::wait_event() => {
                    let Some(event) = media else { return };
                    let mapped = match event {
                        SystemEvent::Play => PlayerCommand::Play,
                        SystemEvent::Pause => PlayerCommand::Pause,
                        SystemEvent::Toggle => PlayerCommand::Toggle,
                        SystemEvent::Next => PlayerCommand::Next,
                        SystemEvent::Prev => PlayerCommand::Previous,
                        SystemEvent::Seek(seconds) => seek_command(seconds),
                        SystemEvent::SetShuffle(on) => PlayerCommand::SetMode {
                            shuffle: Some(on),
                            loop_mode: None,
                        },
                        SystemEvent::SetRepeat(mode) => PlayerCommand::SetMode {
                            shuffle: None,
                            loop_mode: Some(match mode {
                                RepeatMode::Off => LoopMode::None,
                                RepeatMode::Playlist => LoopMode::Queue,
                                RepeatMode::Track => LoopMode::Track,
                            }),
                        },
                    };
                    command(&session, mapped).await;
                }
                received = events.recv() => {
                    match received {
                        Ok((_, ApiEvent::PlayerState(state))) => {
                            let modes = (state.queue.shuffle, state.queue.loop_mode);
                            if last_modes != Some(modes) {
                                last_modes = Some(modes);
                                player::systemint::update_modes(
                                    modes.0,
                                    match modes.1 {
                                        LoopMode::None => RepeatMode::Off,
                                        LoopMode::Queue => RepeatMode::Playlist,
                                        LoopMode::Track => RepeatMode::Track,
                                    },
                                );
                            }
                        }
                        Ok(_) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                    }
                }
            }
        }
    });
}

#[cfg(target_os = "windows")]
pub fn spawn(session: &SessionHandle) {
    use player::systemint::SystemEvent;
    player::systemint::init();
    let session = session.clone();
    tokio::spawn(async move {
        loop {
            let Some(event) = player::systemint::wait_event().await else {
                return;
            };
            let mapped = match event {
                SystemEvent::Play => PlayerCommand::Play,
                SystemEvent::Pause => PlayerCommand::Pause,
                SystemEvent::Toggle => PlayerCommand::Toggle,
                SystemEvent::Next => PlayerCommand::Next,
                SystemEvent::Prev => PlayerCommand::Previous,
                SystemEvent::Seek(seconds) => seek_command(seconds),
            };
            command(&session, mapped).await;
        }
    });
}

#[cfg(target_os = "android")]
pub fn spawn(_session: &SessionHandle) {}
