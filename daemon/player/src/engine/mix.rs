//! A planned DJ-style mix: per-deck curves the decode worker plays over a
//! session's output (speed, fader, three-band cut, optional high-pass).
//!
//! ```text
//!  A  ──────────────[ A's curves for `length` ]| ends
//!  B                [ B's curves from its first sample ... last point held ]──
//!                   ^ A's source time `at`, B's first sample, same output frame
//! ```
//!
//! Curves are sampled every `step` of output time since the deck's mix began;
//! between points they interpolate linearly, past the last point they hold it.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Per-point values of one deck, all of equal length.
#[derive(Clone, Debug, PartialEq)]
pub struct MixCurves {
    /// Playback rate; pitch follows it, like a turntable.
    pub speed: Vec<f32>,
    /// Fader, linear amplitude.
    pub volume: Vec<f32>,
    /// Cuts in dB at the high shelf, mid peak and low shelf; 0 is flat.
    pub high_db: Vec<f32>,
    pub mid_db: Vec<f32>,
    pub bass_db: Vec<f32>,
    /// High-pass corner in Hz per point; 0 or below is off.
    pub highpass_hz: Option<Vec<f32>>,
}

/// Where the deck's three bands sit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MixFilters {
    pub high_hz: f32,
    pub mid_hz: f32,
    pub mid_q: f32,
    pub bass_hz: f32,
}

/// One deck's validated plan.
#[derive(Clone, Debug, PartialEq)]
pub struct DeckMix {
    /// Boxed so a plan keeps `Command` small.
    curves: Box<MixCurves>,
    filters: MixFilters,
    step: Duration,
    length: Duration,
}

/// The incoming side of a planned mix, carried by its `LoadRequest`.
#[derive(Clone, Debug, PartialEq)]
pub struct IncomingMix {
    pub mix: DeckMix,
    /// The session whose `Command::PlanOutgoing` this load answers.
    pub from_token: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MixError {
    /// A curve has no points.
    Empty,
    /// The curves differ in length.
    Uneven,
    /// `step` or `length` is zero.
    Span,
    /// A value is not finite, a speed not positive, or a filter not audible.
    Value,
}

impl std::fmt::Display for MixError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Empty => "mix curve has no points",
            Self::Uneven => "mix curves differ in length",
            Self::Span => "mix step or length is zero",
            Self::Value => "mix value out of range",
        })
    }
}

impl std::error::Error for MixError {}

/// The curves' values at one moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MixPoint {
    pub speed: f32,
    pub volume: f32,
    pub high_db: f32,
    pub mid_db: f32,
    pub bass_db: f32,
    pub highpass_hz: f32,
}

impl DeckMix {
    pub fn new(
        curves: MixCurves,
        filters: MixFilters,
        step: Duration,
        length: Duration,
    ) -> Result<Self, MixError> {
        if step.is_zero() || length.is_zero() {
            return Err(MixError::Span);
        }
        check_curves(&curves)?;
        check_filters(&filters)?;
        Ok(Self {
            curves: Box::new(curves),
            filters,
            step,
            length,
        })
    }

    pub fn length(&self) -> Duration {
        self.length
    }

    pub(crate) fn filters(&self) -> MixFilters {
        self.filters
    }

    /// All curves at `t` seconds into the mix.
    pub(crate) fn point(&self, t: f64) -> MixPoint {
        let curves = &self.curves;
        MixPoint {
            speed: self.sample(&curves.speed, t),
            volume: self.sample(&curves.volume, t),
            high_db: self.sample(&curves.high_db, t),
            mid_db: self.sample(&curves.mid_db, t),
            bass_db: self.sample(&curves.bass_db, t),
            highpass_hz: curves
                .highpass_hz
                .as_deref()
                .map_or(0.0, |curve| self.sample(curve, t)),
        }
    }

    pub(crate) fn speed(&self, t: f64) -> f32 {
        self.sample(&self.curves.speed, t)
    }

    pub(crate) fn volume(&self, t: f64) -> f32 {
        self.sample(&self.curves.volume, t)
    }

    /// Linear between points, the last point past the end. Validation keeps
    /// curves non-empty; the fallback only guards the audio path.
    fn sample(&self, curve: &[f32], t: f64) -> f32 {
        let pos = (t / self.step.as_secs_f64()).max(0.0);
        let index = pos.floor() as usize;
        let Some(&from) = curve.get(index) else {
            return curve.last().copied().unwrap_or(0.0);
        };
        let Some(&to) = curve.get(index + 1) else {
            return from;
        };
        from + (to - from) * (pos - index as f64) as f32
    }
}

fn check_curves(curves: &MixCurves) -> Result<(), MixError> {
    let mut all = vec![
        &curves.speed,
        &curves.volume,
        &curves.high_db,
        &curves.mid_db,
        &curves.bass_db,
    ];
    all.extend(curves.highpass_hz.as_ref());

    let points = curves.speed.len();
    if all.iter().any(|curve| curve.is_empty()) {
        return Err(MixError::Empty);
    }
    if all.iter().any(|curve| curve.len() != points) {
        return Err(MixError::Uneven);
    }
    if all
        .iter()
        .flat_map(|curve| curve.iter())
        .any(|v| !v.is_finite())
    {
        return Err(MixError::Value);
    }
    if curves.speed.iter().any(|speed| *speed <= 0.0) {
        return Err(MixError::Value);
    }
    Ok(())
}

fn check_filters(filters: &MixFilters) -> Result<(), MixError> {
    let values = [
        filters.high_hz,
        filters.mid_hz,
        filters.mid_q,
        filters.bass_hz,
    ];
    if values.iter().all(|v| v.is_finite() && *v > 0.0) {
        Ok(())
    } else {
        Err(MixError::Value)
    }
}

/// The ring index (interleaved samples, `played` scale) where the outgoing
/// deck's mix begins, shared by its worker (writes once), the callback (holds
/// the incoming deck until A's `played` reaches it) and the actor (reports a
/// refusal).
#[derive(Clone, Debug)]
pub(crate) struct MixStart(Arc<AtomicU64>);

const MIX_PENDING: u64 = u64::MAX;
const MIX_REFUSED: u64 = u64::MAX - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MixMark {
    /// The worker has not reached `at` yet.
    Pending,
    /// The worker had already decoded past `at`.
    Refused,
    At(u64),
}

impl MixStart {
    pub(crate) fn pending() -> Self {
        Self(Arc::new(AtomicU64::new(MIX_PENDING)))
    }

    pub(crate) fn refuse(&self) {
        self.0.store(MIX_REFUSED, Ordering::Release);
    }

    /// Stored before the first mixed sample enters the ring, so the callback
    /// sees it by the time it plays that far.
    pub(crate) fn set(&self, index: u64) {
        self.0.store(index.min(MIX_REFUSED - 1), Ordering::Release);
    }

    pub(crate) fn mark(&self) -> MixMark {
        match self.0.load(Ordering::Acquire) {
            MIX_PENDING => MixMark::Pending,
            MIX_REFUSED => MixMark::Refused,
            index => MixMark::At(index),
        }
    }

    /// The `played` count to hold the incoming deck until; never reached
    /// while pending or refused.
    pub(crate) fn target(&self) -> u64 {
        match self.mark() {
            MixMark::At(index) => index,
            MixMark::Pending | MixMark::Refused => u64::MAX,
        }
    }
}

#[cfg(test)]
pub(crate) fn flat_curves(points: usize) -> MixCurves {
    MixCurves {
        speed: vec![1.0; points],
        volume: vec![1.0; points],
        high_db: vec![0.0; points],
        mid_db: vec![0.0; points],
        bass_db: vec![0.0; points],
        highpass_hz: None,
    }
}

#[cfg(test)]
pub(crate) const TEST_FILTERS: MixFilters = MixFilters {
    high_hz: 4_000.0,
    mid_hz: 1_000.0,
    mid_q: 0.5,
    bass_hz: 180.0,
};

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{DeckMix, MixError, MixMark, MixStart, TEST_FILTERS, flat_curves};

    const STEP: Duration = Duration::from_millis(100);
    const LENGTH: Duration = Duration::from_secs(1);

    fn mix_with_volume(volume: Vec<f32>) -> DeckMix {
        let mut curves = flat_curves(volume.len());
        curves.volume = volume;
        DeckMix::new(curves, TEST_FILTERS, STEP, LENGTH).expect("valid mix")
    }

    #[test]
    fn curves_interpolate_between_points() {
        let mix = mix_with_volume(vec![0.0, 1.0, 0.5]);

        assert_eq!(mix.volume(0.0), 0.0);
        assert!((mix.volume(0.05) - 0.5).abs() < 1e-6);
        assert!((mix.volume(0.1) - 1.0).abs() < 1e-6);
        assert!((mix.volume(0.15) - 0.75).abs() < 1e-6);
    }

    #[test]
    fn curves_hold_the_last_point() {
        let mix = mix_with_volume(vec![0.0, 1.0, 0.5]);

        assert_eq!(mix.volume(0.2), 0.5);
        assert_eq!(mix.volume(60.0), 0.5);
        assert_eq!(mix.point(60.0).highpass_hz, 0.0);
    }

    #[test]
    fn invalid_plans_are_rejected() {
        let new = |curves, step, length| DeckMix::new(curves, TEST_FILTERS, step, length);

        assert_eq!(new(flat_curves(0), STEP, LENGTH), Err(MixError::Empty));

        let mut uneven = flat_curves(3);
        uneven.mid_db.pop();
        assert_eq!(new(uneven, STEP, LENGTH), Err(MixError::Uneven));

        let mut highpass = flat_curves(3);
        highpass.highpass_hz = Some(vec![100.0; 2]);
        assert_eq!(new(highpass, STEP, LENGTH), Err(MixError::Uneven));

        assert_eq!(
            new(flat_curves(3), Duration::ZERO, LENGTH),
            Err(MixError::Span)
        );
        assert_eq!(
            new(flat_curves(3), STEP, Duration::ZERO),
            Err(MixError::Span)
        );

        let mut stopped = flat_curves(3);
        stopped.speed[1] = 0.0;
        assert_eq!(new(stopped, STEP, LENGTH), Err(MixError::Value));

        let mut nan = flat_curves(3);
        nan.bass_db[2] = f32::NAN;
        assert_eq!(new(nan, STEP, LENGTH), Err(MixError::Value));

        let mut filters = TEST_FILTERS;
        filters.mid_q = 0.0;
        assert_eq!(
            DeckMix::new(flat_curves(3), filters, STEP, LENGTH),
            Err(MixError::Value)
        );
    }

    #[test]
    fn a_mix_start_reports_what_the_worker_decided() {
        let start = MixStart::pending();
        assert_eq!(start.mark(), MixMark::Pending);
        assert_eq!(start.target(), u64::MAX);

        start.set(88_200);
        assert_eq!(start.mark(), MixMark::At(88_200));
        assert_eq!(start.target(), 88_200);

        start.refuse();
        assert_eq!(start.mark(), MixMark::Refused);
        assert_eq!(start.target(), u64::MAX);
    }
}
