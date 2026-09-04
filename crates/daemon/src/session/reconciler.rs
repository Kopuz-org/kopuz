//! Engine-event reconciliation: token gating, auto-advance, crossfade arming,
//! and the sparse position-anchor projection.

use std::time::Duration;

use api::{Phase as ApiPhase, PlayerState};
use player::engine::{Event as EngineEvent, Phase as EnginePhase};
use tokio::sync::watch;

use super::Session;
use super::state::engine_phase;
use crate::queue_model::NextOutcome;

impl Session {
    pub(super) fn handle_engine_event(
        &mut self,
        event: EngineEvent,
        state_tx: &watch::Sender<PlayerState>,
    ) {
        match event {
            EngineEvent::PhaseChanged {
                token,
                phase: phase @ (EnginePhase::Playing | EnginePhase::Paused),
            } => {
                if token == self.intent.token() {
                    let next_phase = engine_phase(phase);
                    if self.phase != next_phase {
                        self.phase = next_phase;
                        self.publish(state_tx, false);
                        self.publish_position_anchor(
                            state_tx,
                            None,
                            None,
                            phase == EnginePhase::Playing,
                        );
                    }
                } else if phase == EnginePhase::Playing && self.player.session_token() == token {
                    // A session no longer intended is audibly live. Guard on
                    // the live token so a revert seek that outran this event
                    // is not stopped.
                    self.player.stop_for_transition();
                }
            }
            EngineEvent::PhaseChanged {
                token,
                phase: EnginePhase::Idle,
            } if token == self.intent.token() => {
                // Idle from a superseded session must not flicker the state
                // while the intended session keeps playing; the stale-session
                // arms above already handle tearing those down.
                self.phase = ApiPhase::Idle;
                self.publish(state_tx, false);
            }
            EngineEvent::PhaseChanged {
                token,
                phase: EnginePhase::Ended,
            }
            | EngineEvent::Ended { token }
                if token == self.intent.token() =>
            {
                self.phase = ApiPhase::Ended;
                self.record_listen_of_current();
                let _ = self.play_next(false, state_tx);
                self.publish(state_tx, false);
            }
            EngineEvent::TrackSwitched { token, .. }
                if self
                    .pending_transition
                    .as_ref()
                    .is_some_and(|pending| pending.to_token == token) =>
            {
                let committed = self.commit_transition(token);
                debug_assert!(committed);
                self.phase = ApiPhase::Playing;
                self.maybe_record_recent();
                self.publish(state_tx, false);
                self.publish_position_anchor(state_tx, Some(token), None, true);
            }
            EngineEvent::Loaded { token }
                if token != self.intent.token() && self.player.session_token() == token =>
            {
                // A promoted load was superseded or cancelled (including the
                // end-of-queue race). Stop only if it is still the live token.
                self.player.stop_for_transition();
            }
            EngineEvent::Error { token, message } if token == self.intent.token() => {
                tracing::warn!(%message, "engine reported a playback error");
                if self.fail_load(token, message) {
                    self.publish(state_tx, false);
                }
            }
            EngineEvent::Position { token, position }
                if token == self.intent.token() && self.should_arm_crossfade(position) =>
            {
                self.arm_crossfade();
                self.publish(state_tx, false);
            }
            _ => {}
        }
    }

    fn should_arm_crossfade(&self, position: Duration) -> bool {
        if self.phase != ApiPhase::Playing
            || self.intent.is_loading()
            || self.pending_transition.is_some()
            || self.current_track_is_radio()
            || !self.should_crossfade()
            || !self.model.has_next_track()
            || self.armed_transition == Some(self.current_token)
        {
            return false;
        }

        let Some(track) = self.model.current_track() else {
            return false;
        };
        let duration = Duration::from_secs(track.duration);
        let remaining = duration.saturating_sub(position);
        if duration.is_zero()
            || position >= duration
            || remaining > Duration::from_secs(self.config.crossfade_seconds as u64)
        {
            return false;
        }

        true
    }

    fn arm_crossfade(&mut self) {
        let mut candidate = self.model.clone();
        let NextOutcome::Play(idx) = candidate.advance_next() else {
            return;
        };
        self.start_load(idx, true, Some(candidate));
    }
}
