//! State projection: revisioned publishes, the event stream
//! position anchors, and the wire snapshot.

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
            self.refresh_album_context();
            // Only a queue with something in it replaces one that could not be read; a shuffle toggle on nothing does not.
            if !self.model.items().is_empty() {
                self.queue_unread = false;
            }
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

    pub(super) fn album_context_for_token(&self, token: u64) -> bool {
        let position = match self.pending_transition.as_ref() {
            Some(pending) if pending.to_token == token => pending.to_position,
            _ => self.model.current_position(),
        };
        self.model
            .track_at(position)
            .is_some_and(|track| self.model.album_context_at(position, &track.album_id))
    }

    fn refresh_album_context(&self) {
        let visible = self.visible_token();
        self.player
            .set_album_context(visible, self.album_context_for_token(visible));
        if let Some(pending) = &self.pending_transition {
            self.player.set_album_context(
                pending.to_token,
                self.album_context_for_token(pending.to_token),
            );
        }
    }

    /// Sole event egress. A subscriber that falls behind the channel is
    /// told to resync rather than silently losing events; there is no
    /// replay log, because a peer that loses this stream has lost the
    /// process that owns it.
    pub(super) fn emit(&self, event: ApiEvent) {
        let _ = self.events.send(event);
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
        // While an integration owns playback the queue still holds whatever
        // the engine had; the reported track is what is actually audible.
        let shown = match self.external.as_ref() {
            Some(external) => external.track.as_ref(),
            None => self.model.current_track(),
        };
        let track = shown.map(|track| crate::wire::track_info(track, &self.config));
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
            output_latency_ms: Some(self.player.output_latency().as_millis() as u64),
            buffered: self.buffered.clone(),
            fading,
            external: self
                .external
                .as_ref()
                .map(|external| api::ExternalPlayback {
                    kind: external.player.kind().to_string(),
                    device: external.device.clone(),
                }),
            error: self.error.clone(),
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
