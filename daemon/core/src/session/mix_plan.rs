//! Planned mixes: what a transition plugin plans for the track that plays and
//! the one after it, asked off the actor and kept per pair.
//!
//! ```text
//!  track starts ─▶ prefetch_mix ─▶ plugin_transition task ─▶ SessionCmd::MixPlanned
//!                                                              │ kept per (from, to)
//!  Position ─▶ try_arm_mix: plan_outgoing(A) + start_load(B, mix)
//!          └─▶ no plan: the crossfade window as before
//! ```

use std::collections::{HashMap, HashSet, VecDeque};

use player::engine::IncomingMix;

use super::smart_fade::{self, PlannedMix};
use super::*;
use crate::transition::TransitionPlan;

/// Pairs whose answer is kept, oldest out first.
const PLAN_ENTRIES: usize = 64;

/// The outgoing and the incoming track key.
pub(super) type Pair = (String, String);

/// The incoming deck on its way into `start_load`.
pub(super) struct StagedMix {
    pub key: String,
    pub entry: Duration,
    pub length: Duration,
    pub deck: player::engine::DeckMix,
}

#[derive(Default)]
pub(super) struct MixPlans {
    /// `None` for a pair no plugin planned, or whose plan the engine cannot play.
    plans: HashMap<Pair, Option<PlannedMix>>,
    order: VecDeque<Pair>,
    asking: HashSet<Pair>,
    /// The session whose plan the engine will not play, or that came too late.
    pub missed: Option<u64>,
    pub staged: Option<StagedMix>,
    /// The incoming deck of the load with this token.
    pub incoming: Option<(u64, IncomingMix)>,
    /// The load carrying a mix and the session whose plan it answers, until it settles.
    pub loading: Option<(u64, u64)>,
}

impl MixPlans {
    pub fn plan(&self, pair: &Pair) -> Option<&PlannedMix> {
        self.plans.get(pair)?.as_ref()
    }

    /// The staged deck, when it is for the track with `key`.
    pub fn take_staged(&mut self, key: &str) -> Option<StagedMix> {
        self.staged.take().filter(|staged| staged.key == key)
    }

    /// The incoming deck, when it belongs to the load with `token`.
    pub fn take_incoming(&mut self, token: u64) -> Option<IncomingMix> {
        let (owner, _) = self.incoming.as_ref()?;
        if *owner != token {
            return None;
        }
        self.incoming.take().map(|(_, mix)| mix)
    }

    /// The session whose plan the load with `token` answers; clears it.
    pub fn settle(&mut self, token: u64) -> Option<u64> {
        let (owner, from_token) = self.loading?;
        if owner != token {
            return None;
        }
        self.loading = None;
        Some(from_token)
    }

    fn should_ask(&self, pair: &Pair) -> bool {
        !self.plans.contains_key(pair) && !self.asking.contains(pair)
    }

    fn remember(&mut self, pair: Pair, plan: Option<PlannedMix>) {
        self.order.retain(|held| *held != pair);
        self.order.push_back(pair.clone());
        self.plans.insert(pair, plan);
        while self.order.len() > PLAN_ENTRIES {
            if let Some(oldest) = self.order.pop_front() {
                self.plans.remove(&oldest);
            }
        }
    }
}

/// A plugin's plan the engine can play; one it cannot is logged and dropped.
fn playable(plan: Option<TransitionPlan>) -> Option<PlannedMix> {
    match smart_fade::planned_mix(&plan?) {
        Ok(mix) => Some(mix),
        Err(error) => {
            tracing::debug!(%error, "the planned mix cannot play");
            None
        }
    }
}

/// Track keys are what plugins know songs by.
fn pair_of(current: &Track, next: &Track) -> Pair {
    (current.id.key().into_owned(), next.id.key().into_owned())
}

impl Session {
    /// The next queue position and the pair a smart crossfade into it would mix.
    /// Radio and an album that carries on are never mixed.
    pub(super) fn mix_pair(&mut self) -> Option<(usize, Pair)> {
        if !self.settings.playback.smart_crossfade || self.settings.playback.crossfade_seconds == 0
        {
            return None;
        }
        let idx = self.peek_next_index()?;
        let current = self.model.current_track()?;
        let next = self.model.track_at(idx)?;
        let radio = |track: &Track| track.duration == u64::MAX;
        if radio(current) || radio(next) || smart_fade::continues_album(current, next) {
            return None;
        }
        Some((idx, pair_of(current, next)))
    }

    /// Ask the transition plugins once per pair, as soon as both tracks are
    /// known: well before `ASK_BEFORE`, so a slow plugin is not cut short.
    pub(super) fn prefetch_mix(&mut self) {
        let Some((_, pair)) = self.mix_pair() else {
            return;
        };
        if !self.mixes.should_ask(&pair) {
            return;
        }

        self.mixes.asking.insert(pair.clone());
        let token = self.intent.token();
        let tx = self.cmd_tx.clone();
        let materializer = self.materializer.clone();
        tokio::spawn(async move {
            let plan = materializer.plugin_transition(&pair.0, &pair.1).await;
            let _ = tx.send(SessionCmd::MixPlanned {
                token,
                pair,
                plan: plan.map(Box::new),
            });
        });
    }

    /// Keep the plan, and arm at once when the track it is for already plays.
    pub(super) fn handle_mix_planned(
        &mut self,
        token: u64,
        pair: Pair,
        plan: Option<TransitionPlan>,
        state_tx: &watch::Sender<PlayerState>,
    ) {
        self.mixes.asking.remove(&pair);
        self.mixes.remember(pair, playable(plan));

        if token != self.intent.token() {
            return;
        }
        let position = self.player.get_position();
        if self.try_arm_mix(position) {
            self.publish(state_tx, false);
        }
    }

    /// The engine would not mix the load with `token` and started nothing:
    /// the outgoing track plays on and its crossfade window takes over.
    pub(super) fn mix_refused(&mut self, token: u64) -> bool {
        self.mixes.settle(token);
        let PlaybackIntent::Loading { from_token, .. } = self.intent else {
            return false;
        };
        if self.intent.token() != token {
            return false;
        }
        tracing::debug!(token, "the engine refused the planned mix");
        self.mixes.missed = Some(from_token);
        self.pending_transition = None;
        self.set_intent(PlaybackIntent::Committed { token: from_token });
        true
    }

    /// A load carrying a mix that will not come: drop the plan that waits for it.
    pub(super) fn drop_mix_plan(&mut self, token: u64) {
        let Some(from_token) = self.mixes.settle(token) else {
            return;
        };
        self.player.cancel_plan(from_token);
        self.mixes.missed = Some(from_token);
    }
}
