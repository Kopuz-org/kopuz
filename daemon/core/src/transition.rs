//! A plugin's plan for going from one track into the next, as the daemon
//! hands it on: the curves each deck follows, sampled at a fixed step. The
//! engine plays it; this side only asks for it in time and passes it along.

use std::time::Duration;

use tokio::time::Instant;

/// How long before a transition starts its plan is asked for.
pub const ASK_BEFORE: Duration = Duration::from_secs(10);

/// A plan that arrives closer than this to the transition is too late to use.
pub const LATEST_BEFORE: Duration = Duration::from_secs(3);

/// Where the three equalizer bands sit, in Hz, and the mid band's Q.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bands {
    pub high_hz: f32,
    pub mid_hz: f32,
    pub mid_q: f32,
    pub bass_hz: f32,
}

/// One deck's curves, each sampled every [`TransitionPlan::step`] seconds.
#[derive(Debug, Clone, PartialEq)]
pub struct DeckCurves {
    /// Linear gain.
    pub volume: Vec<f32>,
    /// Playback rate.
    pub speed: Vec<f32>,
    pub high_db: Vec<f32>,
    pub mid_db: Vec<f32>,
    pub bass_db: Vec<f32>,
    pub highpass_hz: Option<Vec<f32>>,
}

/// How to go from one track into the next, in seconds. Already checked to be
/// whole (see `plugin_host::transition::check`); whether it mixes well is the
/// engine's to judge.
#[derive(Debug, Clone, PartialEq)]
pub struct TransitionPlan {
    /// Into the outgoing track.
    pub start: f64,
    pub length: f64,
    /// Into the incoming track.
    pub entry: f64,
    pub step: f64,
    pub bands: Bands,
    pub outgoing: DeckCurves,
    pub incoming: DeckCurves,
}

impl From<plugin_host::wit::DeckCurves> for DeckCurves {
    fn from(curves: plugin_host::wit::DeckCurves) -> Self {
        Self {
            volume: curves.volume,
            speed: curves.speed,
            high_db: curves.high_db,
            mid_db: curves.mid_db,
            bass_db: curves.bass_db,
            highpass_hz: curves.highpass_hz,
        }
    }
}

impl From<plugin_host::wit::TransitionPlan> for TransitionPlan {
    fn from(plan: plugin_host::wit::TransitionPlan) -> Self {
        Self {
            start: plan.start,
            length: plan.length,
            entry: plan.entry,
            step: plan.step,
            bands: Bands {
                high_hz: plan.bands.high_hz,
                mid_hz: plan.bands.mid_hz,
                mid_q: plan.bands.mid_q,
                bass_hz: plan.bands.bass_hz,
            },
            outgoing: plan.outgoing.into(),
            incoming: plan.incoming.into(),
        }
    }
}

/// `plan`, unless it came in later than [`LATEST_BEFORE`] ahead of `due`, the
/// moment the transition starts. Asking is never cut short, since that would
/// leave the plugin's instance mid-call; a late answer is only dropped.
pub fn in_time(plan: Option<TransitionPlan>, due: Instant) -> Option<TransitionPlan> {
    let usable = due
        .checked_sub(LATEST_BEFORE)
        .is_some_and(|last| Instant::now() <= last);
    plan.filter(|_| usable)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> TransitionPlan {
        let flat = DeckCurves {
            volume: vec![1.; 3],
            speed: vec![1.; 3],
            high_db: vec![0.; 3],
            mid_db: vec![0.; 3],
            bass_db: vec![0.; 3],
            highpass_hz: None,
        };
        TransitionPlan {
            start: 180.,
            length: 2.,
            entry: 0.,
            step: 1.,
            bands: Bands {
                high_hz: 4_000.,
                mid_hz: 1_000.,
                mid_q: 0.7,
                bass_hz: 120.,
            },
            outgoing: flat.clone(),
            incoming: flat,
        }
    }

    #[test]
    fn a_plan_is_used_only_while_there_is_time_to() {
        let now = Instant::now();
        assert!(in_time(Some(plan()), now + ASK_BEFORE).is_some());
        assert!(in_time(Some(plan()), now + Duration::from_secs(2)).is_none());
        assert!(in_time(None, now + ASK_BEFORE).is_none());
    }
}
