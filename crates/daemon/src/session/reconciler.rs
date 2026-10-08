//! Engine-event reconciliation: token gating, auto-advance, crossfade and
//! gapless arming, and the sparse position-anchor projection.

use std::time::Duration;

use api::{Phase as ApiPhase, PlayerState};
use player::engine::{Event as EngineEvent, Phase as EnginePhase};
use tokio::sync::watch;

use super::state::engine_phase;
use super::{Session, TransitionStage};
use crate::queue_model::NextOutcome;

/// How long before the end a gapless next track starts loading: enough for a
/// remote stream to resolve and probe before the current one drains.
const GAPLESS_PRELOAD: Duration = Duration::from_secs(10);

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
                        let _ = self.arm_gapless_now();
                        self.publish(state_tx, false);
                        self.publish_position_anchor(
                            state_tx,
                            None,
                            None,
                            phase == EnginePhase::Playing,
                        );
                    }
                } else if phase == EnginePhase::Playing
                    && self.player.session_token() == token
                    && !self.is_outgoing(token)
                {
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
                if token == self.intent.token() || self.queued_from(token) =>
            {
                // The engine let go of the queued track (an output rebuild),
                // so the outgoing one ended the ordinary way.
                if token != self.intent.token() {
                    let _ = self.revert_transition();
                }
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
                let queued = self
                    .pending_transition
                    .as_ref()
                    .is_some_and(|pending| pending.stage == TransitionStage::Queued);
                let committed = self.commit_transition(token);
                debug_assert!(committed);
                if queued && let Some(track) = self.model.current_track().cloned() {
                    self.scrobble_committed(track, token);
                }
                self.phase = ApiPhase::Playing;
                self.maybe_record_recent();
                let _ = self.arm_gapless_now();
                self.publish(state_tx, false);
                self.publish_position_anchor(state_tx, Some(token), None, true);
            }
            EngineEvent::Loaded { token }
                if token != self.intent.token()
                    && self.player.session_token() == token
                    && !self.is_outgoing(token) =>
            {
                // A promoted load was superseded or cancelled (including the
                // end-of-queue race). Stop only if it is still the live token.
                self.player.stop_for_transition();
            }
            EngineEvent::Error { token, message }
                if self.pending_transition.as_ref().is_some_and(|pending| {
                    pending.stage == TransitionStage::Queued && pending.to_token == token
                }) =>
            {
                // Nothing of it was heard; the outgoing track plays out and the
                // end-of-track advance retries it and reports a failure.
                tracing::warn!(%message, "the queued gapless track failed");
                let _ = self.revert_transition();
                self.publish(state_tx, false);
            }
            EngineEvent::Error { token, message } if token == self.intent.token() => {
                tracing::warn!(%message, "engine reported a playback error");
                if self.fail_load(token, message) {
                    self.publish(state_tx, false);
                }
            }
            EngineEvent::Position { token, position }
                if token == self.intent.token() && self.should_arm_transition(position) =>
            {
                self.arm_transition();
                self.publish(state_tx, false);
            }
            _ => {}
        }
    }

    /// The session a pending transition keeps playing until the switch; its
    /// events can trail the arming of the next track.
    fn is_outgoing(&self, token: u64) -> bool {
        self.pending_transition
            .as_ref()
            .is_some_and(|pending| pending.from_token == token)
    }

    /// The outgoing session of a queued gapless switch.
    fn queued_from(&self, token: u64) -> bool {
        self.pending_transition.as_ref().is_some_and(|pending| {
            pending.stage == TransitionStage::Queued && pending.from_token == token
        })
    }

    pub(super) fn should_arm_transition(&mut self, position: Duration) -> bool {
        let (window, gapless) = if self.should_crossfade() {
            (
                Duration::from_secs(self.config.crossfade_seconds as u64),
                false,
            )
        } else if self.should_play_gapless() {
            (GAPLESS_PRELOAD, true)
        } else {
            return false;
        };
        if self.phase != ApiPhase::Playing
            || self.intent.is_loading()
            || self.pending_transition.is_some()
            || self.current_track_is_radio()
            || !self.model.has_next_track()
            || self.armed_transition == Some(self.current_token)
        {
            return false;
        }
        // Handing a track to an integration stops the engine, so it waits for
        // the end of this one rather than cutting it short.
        if gapless
            && let NextOutcome::Play(idx) = self.model.peek_next()
            && self
                .model
                .track_at(idx)
                .is_some_and(|track| self.sink_for(track).is_some())
        {
            return false;
        }

        let Some(track) = self.model.current_track() else {
            return false;
        };
        let duration = Duration::from_secs(track.duration);
        let remaining = duration.saturating_sub(position);
        if duration.is_zero() || position >= duration || remaining > window {
            return false;
        }

        true
    }

    pub(super) fn arm_transition(&mut self) {
        let NextOutcome::Play(idx) = self.model.peek_next() else {
            return;
        };
        self.start_load(idx, true);
    }
}
