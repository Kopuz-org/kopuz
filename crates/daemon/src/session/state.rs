//! State projection: revisioned publishes, the sequenced event stream
//! with its replay ring, position anchors, and the wire snapshot.

use super::*;

impl Session {
    pub(super) fn publish(
        &mut self,
        state_tx: &watch::Sender<PlayerState>,
        queue_changed: bool,
    ) -> CommandAck {
        self.rev += 1;
        if queue_changed {
            self.queue_rev = self.rev;
        }
        self.queue_dirty = true;
        let state = self.build_state();
        if queue_changed {
            self.emit(ApiEvent::QueueChanged {
                rev: self.queue_rev,
                length: state.queue.length,
                index: state.queue.index,
            });
        }
        let _ = state_tx.send(state.clone());
        self.emit(ApiEvent::PlayerState(Box::new(state)));
        CommandAck { rev: self.rev }
    }

    /// Sole event egress: stamps the monotonic sequence, records the event in
    /// the replay ring, then broadcasts. SSE ids and `Last-Event-ID` resume
    /// both key off these sequences.
    pub(super) fn emit(&self, event: ApiEvent) {
        let sequence = self.seq.fetch_add(1, Ordering::AcqRel) + 1;
        if let Ok(mut history) = self.history.lock() {
            if history.len() >= EVENT_BUFFER {
                history.pop_front();
            }
            history.push_back((sequence, event.clone()));
        }
        let _ = self.events.send((sequence, event));
    }

    pub(super) fn publish_position_anchor(
        &mut self,
        state_tx: &watch::Sender<PlayerState>,
        token: Option<u64>,
        position: Option<Duration>,
        playing: bool,
    ) {
        let token = token.unwrap_or_else(|| self.visible_token());
        let position = position.unwrap_or_else(|| self.displayed_position());
        let anchor = PositionAnchor {
            ms: position.as_millis() as u64,
            at_ms: self.now_ms(),
            playing,
        };
        self.position = Some(anchor);
        self.queue_dirty = true;
        self.position_token = Some(token);
        let _ = state_tx.send(self.build_state());
        self.emit(ApiEvent::PlayerPosition {
            token,
            position_ms: anchor.ms,
            at_ms: anchor.at_ms,
            playing,
        });
    }

    pub(super) fn visible_token(&self) -> u64 {
        self.pending_transition
            .as_ref()
            .map(|pending| pending.from_token)
            .unwrap_or(self.current_token)
    }

    pub(super) fn displayed_position(&self) -> Duration {
        if self.pending_transition.is_some()
            && let Some(position) = self.player.fading_position()
        {
            return position;
        }
        self.player.get_position()
    }

    pub(super) fn current_track_is_radio(&self) -> bool {
        self.model
            .current_track()
            .is_some_and(|track| track.duration == u64::MAX)
    }

    pub(super) fn should_crossfade(&self) -> bool {
        self.config.crossfade_seconds > 0
            && self.phase == ApiPhase::Playing
            && self.player.can_resume()
    }

    pub(super) fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }

    pub(super) fn build_state(&self) -> PlayerState {
        let track = self.model.current_track().map(now_playing_from);
        let fading = self.pending_transition.as_ref().and_then(|pending| {
            (pending.stage == TransitionStage::Fading).then(|| FadingState {
                from_token: pending.from_token,
                track: track.clone().unwrap_or_default(),
                position_ms: self
                    .player
                    .fading_position()
                    .unwrap_or_default()
                    .as_millis() as u64,
            })
        });
        PlayerState {
            rev: self.rev,
            now_ms: self.now_ms(),
            phase: self.phase,
            intent: self.intent.into(),
            track,
            position: self.position,
            queue: QueueSummary {
                rev: self.queue_rev,
                length: self.model.len() as u32,
                index: (!self.model.is_empty()).then(|| self.model.current_position() as u32),
                shuffle: self.model.shuffle(),
                loop_mode: self.model.loop_mode(),
            },
            volume: self.volume,
            buffered: self.buffered.clone(),
            fading,
            error: self.error.clone(),
            ..Default::default()
        }
    }
}

pub(super) fn engine_phase(phase: EnginePhase) -> ApiPhase {
    match phase {
        EnginePhase::Idle => ApiPhase::Idle,
        EnginePhase::Playing => ApiPhase::Playing,
        EnginePhase::Paused => ApiPhase::Paused,
        EnginePhase::Ended => ApiPhase::Ended,
    }
}

/// Translate the internal track model to the wire summary. The radio duration
/// sentinel is contained at this boundary.
pub(super) fn now_playing_from(track: &Track) -> NowPlaying {
    let radio = track.duration == u64::MAX;
    NowPlaying {
        key: track.id.uid(),
        title: track.title.clone(),
        artist: track.artist.clone(),
        album: track.album.clone(),
        duration_ms: (!radio).then(|| track.duration.saturating_mul(1000)),
        khz: track.khz,
        bitrate: track.bitrate,
        artwork: None,
        kind: if radio {
            TrackKind::Radio
        } else {
            TrackKind::Normal
        },
        seekable: !radio,
    }
}
