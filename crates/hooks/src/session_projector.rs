//! Projects the embedded daemon session's state stream onto the
//! `PlayerController` signals the UI renders from.
//!
//! While Spotify external playback is active the local signals are
//! authoritative and daemon state is ignored; handing control back to the
//! engine re-syncs through `set_queue_tracks`, after which projection
//! resumes.

use std::time::{Duration, Instant};

use api::{ApiEvent, Intent, NowPlaying, Phase, PlayerState};
use dioxus::prelude::*;
use tokio::sync::broadcast::error::RecvError;

use crate::use_player_controller::{BufferedRange, PlayerController};

fn set_if_changed<T: PartialEq + 'static>(signal: &mut Signal<T>, value: T) {
    if *signal.peek() != value {
        signal.set(value);
    }
}

#[derive(Clone, Copy)]
struct DaemonClock {
    daemon_ms: u64,
    local: Instant,
}

impl DaemonClock {
    fn sample(daemon_ms: u64) -> Self {
        Self {
            daemon_ms,
            local: Instant::now(),
        }
    }

    fn local_instant(self, daemon_ms: u64) -> Instant {
        if daemon_ms >= self.daemon_ms {
            self.local
                .checked_add(Duration::from_millis(daemon_ms - self.daemon_ms))
                .unwrap_or(self.local)
        } else {
            self.local
                .checked_sub(Duration::from_millis(self.daemon_ms - daemon_ms))
                .unwrap_or(self.local)
        }
    }
}

/// Translate a daemon-clock position anchor into a local-clock one and the
/// whole-second progress the signal carries.
fn local_anchor(state: &PlayerState, clock: DaemonClock) -> Option<(u64, Instant, bool, u64)> {
    let anchor = state.position?;
    let elapsed_ms = state.now_ms.saturating_sub(anchor.at_ms);
    let instant = clock.local_instant(anchor.at_ms);
    let position_ms = if anchor.playing {
        anchor.ms.saturating_add(elapsed_ms)
    } else {
        anchor.ms
    };
    Some((anchor.ms, instant, anchor.playing, position_ms / 1000))
}

fn visible_track(state: &PlayerState) -> Option<&NowPlaying> {
    state
        .fading
        .as_ref()
        .map(|fading| &fading.track)
        .or(state.track.as_ref())
}

fn apply_state(ctrl: &mut PlayerController, state: PlayerState) -> DaemonClock {
    let clock = DaemonClock::sample(state.now_ms);
    let playing = match state.intent {
        Intent::Loading { .. } => true,
        Intent::Committed { .. } => state.phase == Phase::Playing,
        Intent::Stopped => false,
    };
    set_if_changed(&mut ctrl.is_playing, playing);
    set_if_changed(
        &mut ctrl.loading,
        matches!(state.intent, Intent::Loading { .. }),
    );
    set_if_changed(&mut ctrl.volume, state.volume);
    set_if_changed(
        &mut ctrl.output_latency_ms,
        state.output_latency_ms.unwrap_or(0),
    );
    set_if_changed(
        &mut ctrl.playback_error,
        state.error.as_ref().map(|error| error.message.clone()),
    );

    set_if_changed(&mut ctrl.shuffle, state.queue.shuffle);
    set_if_changed(&mut ctrl.loop_mode, state.queue.loop_mode);
    if let Some(index) = state.queue.index {
        set_if_changed(&mut ctrl.current_queue_index, index as usize);
    }

    // During a crossfade the outgoing track stays on screen and drives the
    // seek bar; otherwise the daemon's selected track does, including a
    // stopped track restored from the previous session.
    let shown = visible_track(&state);
    let fading_secs = state
        .fading
        .as_ref()
        .map(|fading| fading.position_ms as f64 / 1000.0);
    set_if_changed(&mut ctrl.fading_progress, fading_secs);

    match shown {
        Some(now) => {
            set_if_changed(&mut ctrl.current_song_title, now.title.clone());
            set_if_changed(&mut ctrl.current_song_artist, now.artist.clone());
            set_if_changed(&mut ctrl.current_song_album, now.album.clone());
            set_if_changed(&mut ctrl.current_song_khz, now.khz);
            set_if_changed(&mut ctrl.current_song_bitrate, now.bitrate);
            set_if_changed(
                &mut ctrl.current_song_duration,
                now.duration_ms.map(|ms| ms / 1000).unwrap_or(u64::MAX),
            );
            // The wire key is the track's uid (service-prefixed for server
            // tracks); matching by the bare id here missed every server
            // track, leaving the cover and snapshot stale.
            let key_changed = ctrl
                .current_track_snapshot
                .peek()
                .as_ref()
                .is_none_or(|snapshot| snapshot.id.uid() != now.key);
            if key_changed {
                let track = ctrl
                    .queue
                    .peek()
                    .iter()
                    .find(|track| track.id.uid() == now.key)
                    .cloned();
                if let Some(track) = track {
                    ctrl.current_song_cover_url
                        .set(ctrl.cover_url_for_track(&track));
                    ctrl.current_track_snapshot.set(Some(track));
                }
            }
        }
        None => {
            let has_metadata = ctrl.current_track_snapshot.peek().is_some()
                || !ctrl.current_song_title.peek().is_empty()
                || !ctrl.current_song_artist.peek().is_empty()
                || !ctrl.current_song_album.peek().is_empty()
                || *ctrl.current_song_duration.peek() != 0
                || *ctrl.current_song_progress.peek() != 0;
            if has_metadata {
                ctrl.clear_current_track_metadata();
            }
        }
    }

    if state.fading.is_none() {
        match local_anchor(&state, clock) {
            Some((ms, instant, anchor_playing, progress_secs)) => {
                set_if_changed(&mut ctrl.engine_anchor, Some((ms, instant, anchor_playing)));
                set_if_changed(&mut ctrl.current_song_progress, progress_secs);
            }
            None => set_if_changed(&mut ctrl.engine_anchor, None),
        }
    }

    let buffered: Vec<BufferedRange> = state
        .buffered
        .iter()
        .map(|range| BufferedRange {
            start: range.start,
            end: range.end,
            total: range.total.unwrap_or(0),
        })
        .collect();
    set_if_changed(&mut ctrl.buffered_ranges, buffered);
    clock
}

/// A transport command the daemon forwarded because playback is external:
/// route it through the controller, whose methods already speak Spotify
/// while `external_active` is set.
fn apply_external_command(ctrl: &mut PlayerController, command: api::PlayerCommand) {
    use api::PlayerCommand;
    match command {
        PlayerCommand::Play => ctrl.resume(),
        PlayerCommand::Pause => ctrl.pause(),
        PlayerCommand::Toggle => ctrl.toggle(),
        PlayerCommand::Next => ctrl.play_next(),
        PlayerCommand::Previous => ctrl.play_prev(),
        PlayerCommand::Stop => {
            ctrl.stop_external_playback();
            ctrl.is_playing.set(false);
        }
        PlayerCommand::Seek { position_ms } => {
            ctrl.seek(Duration::from_millis(position_ms));
        }
        PlayerCommand::SetVolume { .. } | PlayerCommand::SetMode { .. } => {}
    }
}

fn apply_queue(ctrl: &mut PlayerController, mirror: daemon::QueueMirrorSnapshot) {
    set_if_changed(&mut ctrl.queue, mirror.tracks);
    set_if_changed(&mut ctrl.shuffle_order, mirror.shuffle_order);
    set_if_changed(&mut ctrl.shuffle, mirror.shuffle);
    set_if_changed(&mut ctrl.current_queue_index, mirror.position);
}

pub(crate) fn use_session_projector(ctrl: PlayerController) {
    let mut ctrl = ctrl;
    use_future(move || async move {
        let handle = ctrl.session.peek().clone();
        let mut rx = handle.subscribe();
        let mirror = handle.queue_mirror().await;
        apply_queue(&mut ctrl, mirror);
        let mut daemon_clock = apply_state(&mut ctrl, handle.state());
        loop {
            let ticking = !*ctrl.external_active.peek()
                && *ctrl.is_playing.peek()
                && ctrl.engine_anchor.peek().is_some();
            tokio::select! {
                event = rx.recv() => match event {
                    Ok((_, event)) => {
                        if let ApiEvent::PlayerExternalCommand(command) = event {
                            if *ctrl.external_active.peek() {
                                apply_external_command(&mut ctrl, command);
                            }
                            continue;
                        }
                        if *ctrl.external_active.peek() {
                            continue;
                        }
                        match event {
                            ApiEvent::PlayerState(state) => {
                                daemon_clock = apply_state(&mut ctrl, *state);
                            }
                            ApiEvent::QueueChanged { .. } | ApiEvent::Resync => {
                                let mirror = handle.queue_mirror().await;
                                apply_queue(&mut ctrl, mirror);
                                daemon_clock = apply_state(&mut ctrl, handle.state());
                            }
                            ApiEvent::PlayerPosition { position_ms, at_ms, playing, .. } => {
                                let received_at = Instant::now();
                                let instant = daemon_clock.local_instant(at_ms);
                                let elapsed_ms = if playing {
                                    received_at
                                        .saturating_duration_since(instant)
                                        .as_millis()
                                        .min(u64::MAX as u128) as u64
                                } else {
                                    0
                                };
                                ctrl.engine_anchor
                                    .set(Some((position_ms, instant, playing)));
                                set_if_changed(
                                    &mut ctrl.current_song_progress,
                                    position_ms.saturating_add(elapsed_ms) / 1000,
                                );
                            }
                            ApiEvent::PlayerBuffered { ranges, .. } => {
                                let buffered: Vec<BufferedRange> = ranges
                                    .iter()
                                    .map(|range| BufferedRange {
                                        start: range.start,
                                        end: range.end,
                                        total: range.total.unwrap_or(0),
                                    })
                                    .collect();
                                set_if_changed(&mut ctrl.buffered_ranges, buffered);
                            }
                            _ => {}
                        }
                    }
                    Err(RecvError::Lagged(_)) => {
                        if !*ctrl.external_active.peek() {
                            let mirror = handle.queue_mirror().await;
                            apply_queue(&mut ctrl, mirror);
                            daemon_clock = apply_state(&mut ctrl, handle.state());
                        }
                    }
                    Err(RecvError::Closed) => break,
                },
                _ = tokio::time::sleep(Duration::from_millis(1000)), if ticking => {
                    let progress = ctrl.displayed_progress_secs_f64() as u64;
                    set_if_changed(&mut ctrl.current_song_progress, progress);
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopped_state_keeps_its_selected_track_visible() {
        let state = PlayerState {
            intent: Intent::Stopped,
            track: Some(NowPlaying {
                title: "last played".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        };

        assert_eq!(
            visible_track(&state).map(|track| track.title.as_str()),
            Some("last played")
        );
    }
}
