//! Planned mixes: what a planner (AudioMuse-AI) advises for the track that
//! plays and the one after it, fetched off the actor and kept per pair.
//!
//! ```text
//!  track starts ─▶ prefetch_mix ─▶ planner task ─▶ SessionCmd::MixAdvised
//!                                                    │ cached per (from, to)
//!  Position ─▶ try_arm_mix: plan_outgoing(A) + start_load(B, mix)
//!          └─▶ no plan: crossfade_length fits the fade to the tempos
//! ```

use std::collections::{HashMap, HashSet, VecDeque};

use player::engine::IncomingMix;
use server::audiomuse::TransitionPlan;

use super::smart_fade::{self, PlannedMix};
use super::*;

/// Pairs whose advice is kept, oldest out first.
const ADVICE_ENTRIES: usize = 64;
/// How long a planner that does not offer mixes is left alone, unless the settings change.
const NOT_OFFERED_REST: Duration = Duration::from_secs(30 * 60);

/// What a planner found for one pair of tracks.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct MixAdvice {
    /// The transition, when both songs leave room for one.
    pub plan: Option<TransitionPlan>,
    /// Beats per minute of the outgoing and the incoming song.
    pub tempos: Option<(f32, f32)>,
}

/// Why a planner gave no advice at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MixRefusal {
    /// No planner is configured right now.
    NotSetUp,
    /// The planner's server does not offer mixes; asking again is wasted.
    NotOffered,
}

/// Plans mixes between two tracks by their keys. AudioMuse-AI in the daemon; tests inject a stub.
#[async_trait::async_trait]
pub(crate) trait MixPlanner: Send + Sync {
    async fn advise(&self, from: &str, to: &str) -> Result<MixAdvice, MixRefusal>;
}

/// The outgoing and the incoming track key.
pub(super) type Pair = (String, String);

/// Advice in the engine's terms.
struct Advice {
    mix: Option<PlannedMix>,
    tempos: Option<(f32, f32)>,
}

impl From<MixAdvice> for Advice {
    fn from(advice: MixAdvice) -> Self {
        let mix = advice
            .plan
            .as_ref()
            .and_then(|plan| match smart_fade::planned_mix(plan) {
                Ok(mix) => Some(mix),
                Err(error) => {
                    tracing::debug!(%error, "the planned mix cannot play");
                    None
                }
            });
        Self {
            mix,
            tempos: advice.tempos,
        }
    }
}

/// The incoming deck on its way into `start_load`.
pub(super) struct StagedMix {
    pub key: String,
    pub entry: Duration,
    pub length: Duration,
    pub deck: player::engine::DeckMix,
}

#[derive(Default)]
pub(super) struct MixPlans {
    planner: Option<Arc<dyn MixPlanner>>,
    advice: HashMap<Pair, Advice>,
    order: VecDeque<Pair>,
    asking: HashSet<Pair>,
    resting_until: Option<Instant>,
    /// The session whose plan the engine will not play, or that came too late.
    pub missed: Option<u64>,
    pub staged: Option<StagedMix>,
    /// The incoming deck of the load with this token.
    pub incoming: Option<(u64, IncomingMix)>,
}

impl MixPlans {
    pub fn plan(&self, pair: &Pair) -> Option<&PlannedMix> {
        self.advice.get(pair)?.mix.as_ref()
    }

    fn tempos(&self, pair: &Pair) -> Option<(f32, f32)> {
        self.advice.get(pair)?.tempos
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

    /// A refused planner may have been set up since.
    pub fn wake(&mut self) {
        self.resting_until = None;
    }

    fn should_ask(&self, pair: &Pair) -> bool {
        let resting = self
            .resting_until
            .is_some_and(|until| Instant::now() < until);
        !resting && !self.advice.contains_key(pair) && !self.asking.contains(pair)
    }

    fn remember(&mut self, pair: Pair, advice: Advice) {
        self.order.retain(|held| *held != pair);
        self.order.push_back(pair.clone());
        self.advice.insert(pair, advice);
        while self.order.len() > ADVICE_ENTRIES {
            if let Some(oldest) = self.order.pop_front() {
                self.advice.remove(&oldest);
            }
        }
    }
}

/// Track keys are what the planner knows songs by.
fn pair_of(current: &Track, next: &Track) -> Pair {
    (current.id.key().into_owned(), next.id.key().into_owned())
}

impl Session {
    pub(super) fn set_mix_planner(&mut self, planner: Arc<dyn MixPlanner>) {
        self.mixes.planner = Some(planner);
        self.prefetch_mix();
    }

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

    /// Ask for the mix into the next track once per pair; the answer lands as a command.
    pub(super) fn prefetch_mix(&mut self) {
        let Some(planner) = self.mixes.planner.clone() else {
            return;
        };
        let Some((_, pair)) = self.mix_pair() else {
            return;
        };
        if !self.mixes.should_ask(&pair) {
            return;
        }

        self.mixes.asking.insert(pair.clone());
        let token = self.intent.token();
        let tx = self.cmd_tx.clone();
        tokio::spawn(async move {
            let result = Box::new(planner.advise(&pair.0, &pair.1).await);
            let _ = tx.send(SessionCmd::MixAdvised {
                token,
                pair,
                result,
            });
        });
    }

    /// Keep the advice, and arm at once when the track it is for already plays.
    pub(super) fn handle_mix_advised(
        &mut self,
        token: u64,
        pair: Pair,
        result: Result<MixAdvice, MixRefusal>,
        state_tx: &watch::Sender<PlayerState>,
    ) {
        self.mixes.asking.remove(&pair);
        match result {
            Ok(advice) => self.mixes.remember(pair, advice.into()),
            Err(MixRefusal::NotOffered) => {
                tracing::debug!("the mix planner does not offer mixes; resting");
                self.mixes.resting_until = Some(Instant::now() + NOT_OFFERED_REST);
                return;
            }
            Err(MixRefusal::NotSetUp) => return,
        }

        if token != self.intent.token() {
            return;
        }
        let position = self.player.get_position();
        if self.try_arm_mix(position) {
            self.publish(state_tx, false);
        }
    }

    /// The configured crossfade, fitted to whole bars when both tempos are known.
    pub(super) fn crossfade_length(&self, next: &Track) -> Duration {
        let configured = Duration::from_secs(self.settings.playback.crossfade_seconds as u64);
        if !self.settings.playback.smart_crossfade {
            return configured;
        }
        let tempos = self
            .model
            .current_track()
            .and_then(|current| self.mixes.tempos(&pair_of(current, next)));
        smart_fade::bar_fade(configured, tempos)
    }
}
