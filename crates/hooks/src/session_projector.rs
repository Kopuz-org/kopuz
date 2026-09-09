//! Projects the daemon's state stream onto the `PlayerController` signals
//! the UI renders from.
//!
//! Two snapshots and a stream: the queue and the player state are fetched
//! once, then followed. A resync event means take both again.

use std::time::{Duration, Instant};

use std::sync::Arc;

use api::{ApiEvent, Intent, Phase, PlayerState};
use dioxus::prelude::*;
use futures_util::StreamExt;

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
    // Which device an integration is playing on, so a picker can mark it and
    // the bottombar can say playback is somewhere else.
    set_if_changed(
        &mut ctrl.external_device,
        state
            .external
            .as_ref()
            .and_then(|external| external.device.clone()),
    );

    set_if_changed(&mut ctrl.shuffle, state.queue.shuffle);
    set_if_changed(&mut ctrl.loop_mode, state.queue.loop_mode);
    if let Some(index) = state.queue.index {
        set_if_changed(&mut ctrl.current_queue_index, index as usize);
    }

    // During a crossfade the outgoing track stays on screen and drives the
    // seek bar; otherwise the committed track does.
    let (shown, fading_secs) = match &state.fading {
        Some(fading) => (
            Some(&fading.track),
            Some(fading.position_ms as f64 / 1000.0),
        ),
        None => (state.track.as_ref(), None),
    };
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
            // Match on uid, not key: key is the bare library ref, which is
            // the same string as the uid for local tracks but not for server
            // ones, so matching on it missed every server track and left the
            // cover and snapshot stale.
            let key_changed = ctrl
                .current_track_snapshot
                .peek()
                .as_ref()
                .is_none_or(|snapshot| snapshot.id.uid() != now.uid);
            if key_changed {
                let track = ctrl
                    .queue
                    .peek()
                    .iter()
                    .find(|track| track.id.uid() == now.uid)
                    .cloned();
                if let Some(track) = track {
                    ctrl.current_song_cover_url
                        .set(ctrl.cover_url_for_track(&track));
                    ctrl.current_track_snapshot.set(Some(track));
                }
            }
        }
        None => {
            if state.intent == Intent::Stopped && ctrl.current_track_snapshot.peek().is_some() {
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

/// The queue as the UI mirrors it: the rows in play order, the permutation
/// behind them, and where playback sits.
fn apply_queue(ctrl: &mut PlayerController, snapshot: api::QueueSnapshot) {
    set_if_changed(
        &mut ctrl.queue,
        crate::wire::tracks_from_api(snapshot.items),
    );
    set_if_changed(
        &mut ctrl.shuffle_order,
        snapshot
            .shuffle_order
            .into_iter()
            .map(|index| index as usize)
            .collect(),
    );
    set_if_changed(&mut ctrl.shuffle, snapshot.shuffle);
    set_if_changed(
        &mut ctrl.current_queue_index,
        snapshot.position.unwrap_or(0) as usize,
    );
}

pub(crate) fn use_session_projector(ctrl: PlayerController) {
    let mut ctrl = ctrl;
    use_future(move || async move {
        let api = ctrl.api.peek().clone();
        let mut events = api.events();
        let mut daemon_clock = resync(&mut ctrl, &api).await;
        loop {
            let ticking = *ctrl.is_playing.peek() && ctrl.engine_anchor.peek().is_some();
            tokio::select! {
                event = events.next() => match event {
                    Some(event) => {
                        match event {
                            ApiEvent::PlayerState(state) => {
                                daemon_clock = apply_state(&mut ctrl, *state);
                            }
                            ApiEvent::QueueChanged { .. } | ApiEvent::Resync => {
                                daemon_clock = resync(&mut ctrl, &api).await;
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
                    // The daemon went away: nothing more will arrive, and the
                    // app reports that elsewhere.
                    None => break,
                },
                _ = tokio::time::sleep(Duration::from_millis(1000)), if ticking => {
                    let progress = ctrl.displayed_progress_secs_f64() as u64;
                    set_if_changed(&mut ctrl.current_song_progress, progress);
                }
            }
        }
    });
}

/// Take both snapshots again. Cheap enough to do on any doubt about the
/// mirror, which is what a resync is.
async fn resync(ctrl: &mut PlayerController, api: &Arc<dyn api::KopuzApi>) -> DaemonClock {
    if let Ok(snapshot) = api.queue_snapshot().await {
        apply_queue(ctrl, snapshot);
    }
    match api.player_state().await {
        Ok(state) => apply_state(ctrl, state),
        Err(_) => DaemonClock::sample(0),
    }
}
