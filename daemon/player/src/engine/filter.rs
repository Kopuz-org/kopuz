//! Biquads for a mixing deck, after the RBJ Audio EQ Cookbook: high shelf,
//! peaking, low shelf and a second-order high-pass. The user equalizer in
//! `crate::eq` keeps its own filters.

use std::f32::consts::{FRAC_1_SQRT_2, PI};

/// Gains this close to 0 dB are flat; the filter passes samples unchanged.
const FLAT_DB: f32 = 0.01;
/// Corners are kept under Nyquist, where the cookbook formulas fold over.
const MAX_CORNER: f32 = 0.45;
/// Shelf slope S = 1: the steepest without overshoot.
const SHELF_ALPHA_SCALE: f32 = std::f32::consts::SQRT_2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Coefficients {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Coefficients {
    pub(crate) const IDENTITY: Self = Self {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    /// Divides the cookbook's raw terms by `a0`.
    fn normalized(b: [f32; 3], a: [f32; 3]) -> Self {
        let a0 = if a[0].abs() < 1e-9 { 1e-9 } else { a[0] };
        Self {
            b0: b[0] / a0,
            b1: b[1] / a0,
            b2: b[2] / a0,
            a1: a[1] / a0,
            a2: a[2] / a0,
        }
    }
}

/// `(cos w0, sin w0)` of a corner, clamped under Nyquist.
fn corner(rate: f32, hz: f32) -> (f32, f32) {
    let hz = hz.clamp(1.0, rate * MAX_CORNER);
    let omega = 2.0 * PI * hz / rate;
    (omega.cos(), omega.sin())
}

pub(crate) fn high_shelf(rate: f32, hz: f32, gain_db: f32) -> Coefficients {
    shelf(rate, hz, gain_db, Shelf::High)
}

pub(crate) fn low_shelf(rate: f32, hz: f32, gain_db: f32) -> Coefficients {
    shelf(rate, hz, gain_db, Shelf::Low)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Shelf {
    High,
    Low,
}

fn shelf(rate: f32, hz: f32, gain_db: f32, side: Shelf) -> Coefficients {
    if gain_db.abs() < FLAT_DB {
        return Coefficients::IDENTITY;
    }
    let a = 10.0_f32.powf(gain_db / 40.0);
    let (cos, sin) = corner(rate, hz);
    let lift = 2.0 * a.sqrt() * sin / SHELF_ALPHA_SCALE;

    // The high shelf is the low shelf with the cosine terms' signs flipped.
    let sign = if side == Shelf::Low { 1.0 } else { -1.0 };
    let b = [
        a * ((a + 1.0) - sign * (a - 1.0) * cos + lift),
        sign * 2.0 * a * ((a - 1.0) - sign * (a + 1.0) * cos),
        a * ((a + 1.0) - sign * (a - 1.0) * cos - lift),
    ];
    let a_terms = [
        (a + 1.0) + sign * (a - 1.0) * cos + lift,
        -sign * 2.0 * ((a - 1.0) + sign * (a + 1.0) * cos),
        (a + 1.0) + sign * (a - 1.0) * cos - lift,
    ];
    Coefficients::normalized(b, a_terms)
}

pub(crate) fn peaking(rate: f32, hz: f32, q: f32, gain_db: f32) -> Coefficients {
    if gain_db.abs() < FLAT_DB {
        return Coefficients::IDENTITY;
    }
    let a = 10.0_f32.powf(gain_db / 40.0);
    let (cos, sin) = corner(rate, hz);
    let alpha = sin / (2.0 * q.max(0.001));
    Coefficients::normalized(
        [1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a],
        [1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a],
    )
}

/// Butterworth (Q = 1/sqrt 2) high-pass; a corner of 0 Hz or below is off.
pub(crate) fn high_pass(rate: f32, hz: f32) -> Coefficients {
    if hz <= 0.0 {
        return Coefficients::IDENTITY;
    }
    let (cos, sin) = corner(rate, hz);
    let alpha = sin / (2.0 * FRAC_1_SQRT_2);
    let pass = (1.0 + cos) / 2.0;
    Coefficients::normalized(
        [pass, -(1.0 + cos), pass],
        [1.0 + alpha, -2.0 * cos, 1.0 - alpha],
    )
}

/// Transposed direct form II; coefficients may change between samples.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Biquad {
    pub coeffs: Coefficients,
    z1: f32,
    z2: f32,
}

impl Default for Biquad {
    fn default() -> Self {
        Self {
            coeffs: Coefficients::IDENTITY,
            z1: 0.0,
            z2: 0.0,
        }
    }
}

impl Biquad {
    pub(crate) fn process(&mut self, sample: f32) -> f32 {
        let c = self.coeffs;
        let output = c.b0 * sample + self.z1;
        self.z1 = c.b1 * sample - c.a1 * output + self.z2;
        self.z2 = c.b2 * sample - c.a2 * output;
        output
    }
}

#[cfg(test)]
mod tests {
    use super::{Biquad, Coefficients, high_pass, high_shelf, low_shelf, peaking};

    const RATE: f32 = 44_100.0;

    /// RMS of a sine at `hz` after the filter, past its settling time.
    fn rms_through(coeffs: Coefficients, hz: f32) -> f32 {
        let mut filter = Biquad {
            coeffs,
            ..Biquad::default()
        };
        let frames = RATE as usize;
        let settle = frames / 2;
        let mut sum = 0.0;
        for n in 0..frames {
            let x = (2.0 * std::f32::consts::PI * hz * n as f32 / RATE).sin();
            let y = filter.process(x);
            if n >= settle {
                sum += y * y;
            }
        }
        (sum / (frames - settle) as f32).sqrt()
    }

    const SINE_RMS: f32 = std::f32::consts::FRAC_1_SQRT_2;

    #[test]
    fn flat_filters_pass_samples_unchanged() {
        for coeffs in [
            high_shelf(RATE, 4_000.0, 0.0),
            peaking(RATE, 1_000.0, 0.5, 0.0),
            low_shelf(RATE, 180.0, 0.0),
            high_pass(RATE, 0.0),
        ] {
            assert_eq!(coeffs, Coefficients::IDENTITY);
            let mut filter = Biquad {
                coeffs,
                ..Biquad::default()
            };
            for x in [0.25, -0.5, 0.999, 0.0, -1.0] {
                assert_eq!(filter.process(x), x);
            }
        }
    }

    #[test]
    fn a_deep_low_shelf_cuts_the_bass_and_keeps_the_treble() {
        let coeffs = low_shelf(RATE, 180.0, -40.0);

        assert!(rms_through(coeffs, 40.0) < SINE_RMS * 0.05);
        assert!((rms_through(coeffs, 5_000.0) - SINE_RMS).abs() < SINE_RMS * 0.05);
    }

    #[test]
    fn a_deep_high_shelf_cuts_the_treble_and_keeps_the_bass() {
        let coeffs = high_shelf(RATE, 4_000.0, -40.0);

        assert!(rms_through(coeffs, 16_000.0) < SINE_RMS * 0.05);
        assert!((rms_through(coeffs, 100.0) - SINE_RMS).abs() < SINE_RMS * 0.05);
    }

    #[test]
    fn a_peak_cut_dips_its_band_only() {
        let coeffs = peaking(RATE, 1_000.0, 0.5, -40.0);

        assert!(rms_through(coeffs, 1_000.0) < SINE_RMS * 0.05);
        assert!((rms_through(coeffs, 20.0) - SINE_RMS).abs() < SINE_RMS * 0.1);
    }

    #[test]
    fn the_high_pass_removes_dc_and_low_tones() {
        let coeffs = high_pass(RATE, 1_000.0);

        let mut filter = Biquad {
            coeffs,
            ..Biquad::default()
        };
        let mut last = 1.0;
        for _ in 0..RATE as usize {
            last = filter.process(1.0);
        }
        assert!(last.abs() < 1e-4, "dc leaks: {last}");

        assert!(rms_through(coeffs, 50.0) < SINE_RMS * 0.01);
        assert!((rms_through(coeffs, 10_000.0) - SINE_RMS).abs() < SINE_RMS * 0.05);
    }
}
