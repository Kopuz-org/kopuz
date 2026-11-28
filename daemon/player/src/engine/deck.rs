//! One mixing deck in the decode worker: plays a `DeckMix` over the session's
//! output-rate stream. Per output frame it reads the input at the speed
//! curve's rate (pitch moves with tempo), runs the three-band cut and the
//! optional high-pass, then the fader.
//!
//! ```text
//!  input ─▶ variable-rate read ─▶ high shelf ─▶ peak ─▶ low shelf ─▶ high-pass ─▶ × fader ─▶ ring
//!           (pos += speed(t))     coefficients from the curves every BLOCK_FRAMES
//! ```
//!
//! `t` is output time since the deck's mix began. Steady state allocates
//! nothing: the input carry and the caller's output buffer keep their capacity.

use super::filter::{self, Biquad};
use super::mix::DeckMix;

/// Output frames between filter coefficient updates (~3 ms at 44.1 kHz).
const BLOCK_FRAMES: u64 = 128;
/// Input frames of carry reserved up front; a decoded packet rarely exceeds it.
const CARRY_FRAMES: usize = 8_192;

/// Which side of the mix a deck plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    /// Ends after the mix's length: the outgoing track stops with the mix.
    Outgoing,
    /// Plays on past the length with the last point held.
    Incoming,
}

/// The four filters of one channel, in signal order.
#[derive(Clone, Copy, Default)]
struct Strip {
    high: Biquad,
    mid: Biquad,
    bass: Biquad,
    high_pass: Biquad,
}

impl Strip {
    fn process(&mut self, sample: f32) -> f32 {
        let sample = self.high.process(sample);
        let sample = self.mid.process(sample);
        let sample = self.bass.process(sample);
        self.high_pass.process(sample)
    }
}

pub(crate) struct Deck {
    mix: DeckMix,
    side: Side,
    rate: f64,
    channels: usize,
    /// Output frames produced since the mix began.
    elapsed: u64,
    /// Where the outgoing deck stops, in output frames.
    end: u64,
    /// Interleaved input not yet consumed; `pos` is the read position into it, in frames.
    carry: Vec<f32>,
    pos: f64,
    /// Input frames read beyond the output frames produced; the speed curve
    /// moves the track's timeline this far from what the ring counts.
    drift: f64,
    strips: Vec<Strip>,
    /// The fractional read position was dropped once the held speed is 1, so
    /// the rest of the track copies samples instead of interpolating them.
    settled: bool,
}

impl Deck {
    pub(crate) fn new(mix: DeckMix, side: Side, rate: u32, channels: usize) -> Self {
        let channels = channels.max(1);
        let rate = f64::from(rate.max(1));
        let end = (mix.length().as_secs_f64() * rate).round() as u64;
        Self {
            mix,
            side,
            rate,
            channels,
            elapsed: 0,
            end,
            carry: Vec::with_capacity(CARRY_FRAMES * channels),
            pos: 0.0,
            drift: 0.0,
            strips: vec![Strip::default(); channels],
            settled: false,
        }
    }

    /// The outgoing deck has played its whole mix.
    pub(crate) fn done(&self) -> bool {
        self.side == Side::Outgoing && self.elapsed >= self.end
    }

    /// The incoming deck is past its mix and holds the last point.
    pub(crate) fn holding(&self) -> bool {
        self.side == Side::Incoming && self.elapsed >= self.end
    }

    /// Input frames read beyond the output frames produced so far.
    pub(crate) fn drift(&self) -> f64 {
        self.drift
    }

    /// Mixes `input` (interleaved, output rate) into `out`, which is cleared
    /// first. Input the speed curve has not reached yet carries over.
    pub(crate) fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        out.clear();
        self.carry.extend_from_slice(input);

        while !self.done() && self.render_block(out) {}

        // Drop the frames the read position has moved past.
        let frames = self.carry.len() / self.channels;
        let consumed = (self.pos.floor() as usize).min(frames);
        self.carry.drain(..consumed * self.channels);
        self.pos -= consumed as f64;
    }

    fn seconds(&self) -> f64 {
        self.elapsed as f64 / self.rate
    }

    /// Past the mix with the speed held at exactly 1, a leftover fraction
    /// would interpolate (and dull) the rest of the track. Snapping it moves
    /// the read point by under half a sample.
    fn settle(&mut self) {
        if self.settled || self.elapsed < self.end || self.mix.speed(self.seconds()) != 1.0 {
            return;
        }
        let snapped = self.pos.round();
        self.drift += snapped - self.pos;
        self.pos = snapped;
        self.settled = true;
    }

    /// Filter coefficients for the block starting now.
    fn tune(&mut self) {
        let point = self.mix.point(self.seconds());
        let bands = self.mix.filters();
        let rate = self.rate as f32;
        let high = filter::high_shelf(rate, bands.high_hz, point.high_db);
        let mid = filter::peaking(rate, bands.mid_hz, bands.mid_q, point.mid_db);
        let bass = filter::low_shelf(rate, bands.bass_hz, point.bass_db);
        let high_pass = filter::high_pass(rate, point.highpass_hz);
        for strip in &mut self.strips {
            strip.high.coeffs = high;
            strip.mid.coeffs = mid;
            strip.bass.coeffs = bass;
            strip.high_pass.coeffs = high_pass;
        }
    }

    /// Up to `BLOCK_FRAMES` output frames; false once the input runs out.
    fn render_block(&mut self, out: &mut Vec<f32>) -> bool {
        let channels = self.channels;
        let frames = self.carry.len() / channels;
        let block_end = self.elapsed - self.elapsed % BLOCK_FRAMES + BLOCK_FRAMES;

        while self.elapsed < block_end && !self.done() {
            // Both follow output time, so where a packet ends cannot move them.
            if self.elapsed.is_multiple_of(BLOCK_FRAMES) {
                self.tune();
            }
            self.settle();

            let index = self.pos.floor() as usize;
            let frac = (self.pos - index as f64) as f32;
            // Interpolating needs the next frame too; a whole position does not.
            let needed = if frac == 0.0 { index + 1 } else { index + 2 };
            if needed > frames {
                return false;
            }

            let t = self.seconds();
            let volume = self.mix.volume(t);
            for (ch, strip) in self.strips.iter_mut().enumerate() {
                let here = self.carry[index * channels + ch];
                let sample = if frac == 0.0 {
                    here
                } else {
                    here + (self.carry[(index + 1) * channels + ch] - here) * frac
                };
                out.push(strip.process(sample) * volume);
            }

            let speed = f64::from(self.mix.speed(t));
            self.pos += speed;
            self.drift += speed - 1.0;
            self.elapsed += 1;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{Deck, Side};
    use crate::engine::mix::{DeckMix, MixCurves, TEST_FILTERS, flat_curves};

    const RATE: u32 = 44_100;
    const STEP: Duration = Duration::from_millis(100);

    fn deck(curves: MixCurves, length: Duration, side: Side) -> Deck {
        let mix = DeckMix::new(curves, TEST_FILTERS, STEP, length).expect("valid mix");
        Deck::new(mix, side, RATE, 2)
    }

    /// A stereo signal that never repeats a value, so any shift shows.
    fn signal(frames: usize) -> Vec<f32> {
        (0..frames * 2)
            .map(|i| ((i as f32) * 0.001).sin() * 0.5)
            .collect()
    }

    #[test]
    fn a_flat_deck_at_speed_one_is_bit_exact() {
        let mut deck = deck(flat_curves(10), Duration::from_secs(1), Side::Incoming);
        let input = signal(5_000);
        let mut out = Vec::new();
        let mut all = Vec::new();

        for chunk in input.chunks(1_152 * 2) {
            deck.process(chunk, &mut out);
            all.extend_from_slice(&out);
        }

        assert_eq!(all, input);
    }

    #[test]
    fn a_faster_deck_reads_more_input_per_output_second() {
        let mut curves = flat_curves(10);
        curves.speed = vec![1.02; 10];
        let mut deck = deck(curves, Duration::from_secs(10), Side::Incoming);
        let input = signal(RATE as usize);
        let mut out = Vec::new();
        let mut produced = 0;

        for chunk in input.chunks(1_024 * 2) {
            deck.process(chunk, &mut out);
            produced += out.len() / 2;
        }

        let expected = RATE as f64 / 1.02;
        assert!(
            (produced as f64 - expected).abs() < 2.0,
            "{produced} frames out of one second, expected {expected}"
        );
    }

    #[test]
    fn the_outgoing_deck_stops_after_its_length() {
        let mut deck = deck(flat_curves(10), Duration::from_millis(500), Side::Outgoing);
        let mut out = Vec::new();
        let mut produced = 0;

        for chunk in signal(RATE as usize).chunks(4_096) {
            deck.process(chunk, &mut out);
            produced += out.len() / 2;
        }

        assert_eq!(produced, RATE as usize / 2);
        assert!(deck.done());
    }

    #[test]
    fn the_fader_follows_the_volume_curve() {
        let mut curves = flat_curves(2);
        curves.volume = vec![0.0, 1.0];
        let mut deck = deck(curves, Duration::from_secs(1), Side::Incoming);
        let mut out = Vec::new();

        deck.process(&vec![1.0; 2 * RATE as usize / 5], &mut out);

        assert_eq!(out[0], 0.0);
        let mid = (RATE as usize / 20) * 2;
        assert!((out[mid] - 0.5).abs() < 1e-3, "{}", out[mid]);
        assert_eq!(*out.last().unwrap(), 1.0);
    }

    #[test]
    fn a_held_speed_of_one_after_the_mix_copies_samples_again() {
        let mut curves = flat_curves(2);
        curves.speed = vec![1.5, 1.0];
        let mut deck = deck(curves, Duration::from_millis(100), Side::Incoming);
        let input = signal(RATE as usize);
        let mut out = Vec::new();

        for chunk in input.chunks(1_024 * 2) {
            deck.process(chunk, &mut out);
        }
        deck.process(&input[..2_048], &mut out);

        assert_eq!(deck.pos.fract(), 0.0);
    }

    /// Plays `input` through a fresh deck in packets of `packet` frames.
    fn played_in_packets(curves: &MixCurves, input: &[f32], packet: usize) -> Vec<f32> {
        let mut deck = deck(curves.clone(), Duration::from_millis(100), Side::Incoming);
        let mut out = Vec::new();
        let mut all = Vec::new();
        for chunk in input.chunks(packet * 2) {
            deck.process(chunk, &mut out);
            all.extend_from_slice(&out);
        }
        all
    }

    fn assert_same_audio(a: &[f32], b: &[f32]) {
        assert!(!a.is_empty());
        assert_eq!(a.len(), b.len(), "the packet size changed the length");
        let first = (0..a.len()).find(|&i| (a[i] - b[i]).abs() > 1e-6);
        assert_eq!(first, None, "the packet size changed the audio");
    }

    #[test]
    fn packet_size_does_not_move_the_coefficient_updates() {
        let mut curves = flat_curves(2);
        curves.mid_db = vec![0.0, -40.0];
        let input = signal(8_000);

        assert_same_audio(
            &played_in_packets(&curves, &input, 8_000),
            &played_in_packets(&curves, &input, 64),
        );
    }

    #[test]
    fn packet_size_does_not_move_where_the_deck_settles() {
        let mut curves = flat_curves(2);
        curves.speed = vec![1.5, 1.0];
        let input = signal(16_000);

        assert_same_audio(
            &played_in_packets(&curves, &input, 16_000),
            &played_in_packets(&curves, &input, 100),
        );
    }
}
