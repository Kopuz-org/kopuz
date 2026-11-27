//! What the host checks of a plugin's transition plan before kopuz looks at
//! it: that it is whole, and small enough to carry. Whether its curves make a
//! good mix is the engine's to judge.

use crate::wit::{DeckCurves, TransitionPlan};

/// The most samples one curve may hold, so a plugin cannot hand over megabytes:
/// a minute at a hundredth of a second, with room to spare.
pub const MAX_SAMPLES: usize = 8_192;

/// Why a plan was turned away.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PlanError {
    #[error("{0} is not a number")]
    NotFinite(&'static str),
    #[error("{0} is below zero")]
    Negative(&'static str),
    #[error("the step has to be above zero")]
    NoStep,
    #[error("a transition of {samples} samples a curve is too long")]
    TooLong { samples: usize },
    #[error("{0} has no samples")]
    Empty(&'static str),
    #[error("{deck} {curve} has {found} samples, not {wanted}")]
    WrongLength {
        deck: &'static str,
        curve: &'static str,
        found: usize,
        wanted: usize,
    },
}

/// How many samples a curve covering `length` seconds at `step` holds: the
/// most a plan needs, since past its last sample a curve holds that value.
pub fn samples(length: f64, step: f64) -> usize {
    (length / step).round() as usize + 1
}

/// Whether `plan` is whole, as the engine takes it: every number finite, the
/// times not negative, the step above zero, and each deck's curves not empty
/// and of one length, at most [`MAX_SAMPLES`].
pub fn check(plan: &TransitionPlan) -> Result<(), PlanError> {
    for (name, value) in [
        ("start", plan.start),
        ("length", plan.length),
        ("entry", plan.entry),
        ("step", plan.step),
    ] {
        if !value.is_finite() {
            return Err(PlanError::NotFinite(name));
        }
        if value < 0. {
            return Err(PlanError::Negative(name));
        }
    }
    if plan.step <= 0. {
        return Err(PlanError::NoStep);
    }
    let bands = &plan.bands;
    for (name, value) in [
        ("high_hz", bands.high_hz),
        ("mid_hz", bands.mid_hz),
        ("mid_q", bands.mid_q),
        ("bass_hz", bands.bass_hz),
    ] {
        if !value.is_finite() {
            return Err(PlanError::NotFinite(name));
        }
    }
    deck("outgoing", &plan.outgoing)?;
    deck("incoming", &plan.incoming)
}

fn deck(deck: &'static str, curves: &DeckCurves) -> Result<(), PlanError> {
    let wanted = curves.volume.len();
    if wanted == 0 {
        return Err(PlanError::Empty(deck));
    }
    if wanted > MAX_SAMPLES {
        return Err(PlanError::TooLong { samples: wanted });
    }
    let mut all = vec![
        ("volume", &curves.volume),
        ("speed", &curves.speed),
        ("high_db", &curves.high_db),
        ("mid_db", &curves.mid_db),
        ("bass_db", &curves.bass_db),
    ];
    if let Some(highpass) = &curves.highpass_hz {
        all.push(("highpass_hz", highpass));
    }
    for (curve, samples) in all {
        if samples.len() != wanted {
            return Err(PlanError::WrongLength {
                deck,
                curve,
                found: samples.len(),
                wanted,
            });
        }
        if samples.iter().any(|sample| !sample.is_finite()) {
            return Err(PlanError::NotFinite(curve));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wit::Bands;

    fn flat(samples: usize, volume: f32) -> DeckCurves {
        DeckCurves {
            volume: vec![volume; samples],
            speed: vec![1.; samples],
            high_db: vec![0.; samples],
            mid_db: vec![0.; samples],
            bass_db: vec![0.; samples],
            highpass_hz: None,
        }
    }

    fn plan(length: f64, step: f64) -> TransitionPlan {
        let samples = samples(length, step);
        TransitionPlan {
            start: 180.,
            length,
            entry: 0.,
            step,
            bands: Bands {
                high_hz: 4_000.,
                mid_hz: 1_000.,
                mid_q: 0.7,
                bass_hz: 120.,
            },
            outgoing: flat(samples, 1.),
            incoming: flat(samples, 0.),
        }
    }

    #[test]
    fn a_whole_plan_passes() {
        assert_eq!(samples(8., 0.5), 17);
        assert_eq!(check(&plan(8., 0.5)), Ok(()));
    }

    #[test]
    fn a_curve_of_the_wrong_length_is_turned_away() {
        let mut short = plan(8., 0.5);
        short.incoming.bass_db.pop();
        assert_eq!(
            check(&short),
            Err(PlanError::WrongLength {
                deck: "incoming",
                curve: "bass_db",
                found: 16,
                wanted: 17,
            })
        );
        let mut highpass = plan(8., 0.5);
        highpass.incoming.highpass_hz = Some(vec![200.; 3]);
        assert!(matches!(
            check(&highpass),
            Err(PlanError::WrongLength {
                curve: "highpass_hz",
                ..
            })
        ));
    }

    #[test]
    fn numbers_that_are_not_or_go_backwards_are_turned_away() {
        let mut nan = plan(8., 0.5);
        nan.outgoing.volume[3] = f32::NAN;
        assert_eq!(check(&nan), Err(PlanError::NotFinite("volume")));
        let mut back = plan(8., 0.5);
        back.entry = -1.;
        assert_eq!(check(&back), Err(PlanError::Negative("entry")));
        let mut still = plan(8., 0.5);
        still.step = 0.;
        assert_eq!(check(&still), Err(PlanError::NoStep));
    }

    #[test]
    fn fewer_points_than_the_step_allows_are_fine_and_none_are_not() {
        let mut sparse = plan(8., 0.5);
        for deck in [&mut sparse.outgoing, &mut sparse.incoming] {
            for curve in [
                &mut deck.volume,
                &mut deck.speed,
                &mut deck.high_db,
                &mut deck.mid_db,
                &mut deck.bass_db,
            ] {
                curve.truncate(3);
            }
        }
        assert_eq!(
            check(&sparse),
            Ok(()),
            "the last point holds, as the engine plays it"
        );
        let mut empty = plan(8., 0.5);
        for curve in [
            &mut empty.incoming.volume,
            &mut empty.incoming.speed,
            &mut empty.incoming.high_db,
            &mut empty.incoming.mid_db,
            &mut empty.incoming.bass_db,
        ] {
            curve.clear();
        }
        assert_eq!(check(&empty), Err(PlanError::Empty("incoming")));
    }

    #[test]
    fn a_plan_too_long_to_carry_is_turned_away() {
        assert!(matches!(
            check(&plan(600., 0.01)),
            Err(PlanError::TooLong { .. })
        ));
    }
}
