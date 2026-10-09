//! A track's other cut: learning it from the source, and swapping the
//! playing track for it at the same place in the song.

use super::*;

impl Session {
    /// Ask the source, once per track, for the cut it pairs with a track
    /// that is about to play and does not say. The answer lands as
    /// [`SessionCmd::CounterpartFound`].
    pub(super) fn ask_counterpart(&mut self, track: &Track) {
        if track.counterpart.is_some() || track.id.service().is_none() {
            return;
        }
        let Some(source) = self.active_source.clone() else {
            return;
        };
        if !source.capabilities().music_videos {
            return;
        }
        let uid = track.id.uid();
        if let Some(known) = self.counterparts.get(&uid) {
            if let Some(counterpart) = known.clone() {
                let _ = self.cmd_tx.send(SessionCmd::CounterpartFound {
                    uid,
                    counterpart: Box::new(counterpart),
                });
            }
            return;
        }
        self.counterparts.insert(uid, None);
        let tx = self.cmd_tx.clone();
        let track = track.clone();
        tokio::spawn(async move {
            match source.counterpart(&track).await {
                Ok(Some(counterpart)) => {
                    let _ = tx.send(SessionCmd::CounterpartFound {
                        uid: track.id.uid(),
                        counterpart: Box::new(counterpart),
                    });
                }
                Ok(None) => {}
                Err(error) => tracing::debug!(%error, "no counterpart lookup for this track"),
            }
        });
    }

    /// Record a counterpart on every queued copy of the track that lacks one.
    pub(super) fn learn_counterpart(
        &mut self,
        uid: &str,
        counterpart: reader::Counterpart,
    ) -> bool {
        let mut changed = false;
        for position in 0..self.model.len() {
            if let Some(track) = self.model.track_at_mut(position)
                && track.counterpart.is_none()
                && track.id.uid() == uid
            {
                track.counterpart = Some(Box::new(counterpart.clone()));
                changed = true;
            }
        }
        if changed {
            self.queue_dirty = true;
        }
        changed
    }

    /// Swap the current track for its `version` cut, mapping the position
    /// through the pair's shared stretches. Playing, the other cut loads
    /// there and plays on; otherwise it waits there for the next play.
    pub(super) fn set_version(
        &mut self,
        version: api::TrackVersion,
        state_tx: &watch::Sender<PlayerState>,
    ) -> Result<bool, ApiError> {
        let idx = self.model.current_position();
        let track = self
            .model
            .track_at(idx)
            .cloned()
            .ok_or_else(|| ApiError::invalid_input("nothing is queued"))?;
        let counterpart = track
            .counterpart
            .as_deref()
            .ok_or_else(|| ApiError::invalid_input("this track has no other version"))?;
        if counterpart.video != (version == api::TrackVersion::Video) {
            return Ok(false);
        }
        let other = track
            .counterpart_track()
            .ok_or_else(|| ApiError::invalid_input("this track has no other version"))?;
        // A crossfade into the next track is abandoned: the swap is about
        // the track still playing.
        self.revert_transition();
        let playing = self.phase == ApiPhase::Playing || self.intent.is_loading();
        let position_ms = if self.phase == ApiPhase::Playing && !self.intent.is_loading() {
            self.displayed_position().as_millis() as u64
        } else {
            self.position.map(|anchor| anchor.ms).unwrap_or_default()
        };
        let own_ms = (track.duration > 0 && track.duration != u64::MAX)
            .then(|| track.duration.saturating_mul(1000));
        let mapped_ms = counterpart.map_position(position_ms, own_ms);
        tracing::info!(
            from = %track.id.uid(),
            to = %other.id.uid(),
            position_ms,
            mapped_ms,
            "switching versions"
        );
        self.pending_resume = Some(PendingResumeState {
            track_key: other.id.uid(),
            position_ms: mapped_ms,
        });
        if let Some(slot) = self.model.track_at_mut(idx) {
            *slot = other;
        }
        self.queue_dirty = true;
        // The swap carries on the listen in progress, so the other cut is not
        // recorded as a play of its own when it commits.
        self.last_recent_key = self.model.current_track().map(|track| track.id.uid());
        if playing {
            if !self.start_load(idx, false) {
                return Err(Self::unavailable_track_error());
            }
        } else {
            self.cancel_load_task();
            self.player.stop_for_transition();
            self.set_intent(PlaybackIntent::Stopped);
            self.phase = ApiPhase::Idle;
            self.buffered.clear();
            self.publish_position_anchor(
                state_tx,
                Some(0),
                Some(Duration::from_millis(mapped_ms)),
                false,
            );
        }
        Ok(true)
    }
}
