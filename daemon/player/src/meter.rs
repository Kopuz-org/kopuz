//! What the music sounds like right now, for whatever follows it: the mix's
//! loudness and peak and three bands, over short windows. The audio thread
//! measures only while someone asks, and publishes each window through atomics.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// How much audio one published window covers.
pub const WINDOW_SECONDS: f32 = 0.05;

/// Windows kept for a reader that missed some: four tenths of a second.
const KEPT: usize = 8;

/// Windows a reader can be handed. The slot after the newest is the one the
/// audio thread writes next, so it is never read.
const READABLE: u64 = KEPT as u64 - 1;

/// Where the low band ends and the high band starts.
const LOW_HZ: f32 = 200.0;
const HIGH_HZ: f32 = 2_000.0;

/// One window, each an RMS from 0 (silence) towards 1 (full scale), except `peak`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Levels {
    pub level: f32,
    /// The largest sample, from 0 to 1.
    pub peak: f32,
    /// Below about 200 Hz: kick drums and bass.
    pub low: f32,
    pub mid: f32,
    /// Above about 2 kHz: cymbals and hi-hats.
    pub high: f32,
}

/// Shared between the audio thread and whoever reads it.
#[derive(Debug, Default)]
pub struct AudioMeter {
    wanted: AtomicBool,
    windows: AtomicU64,
    /// The last [`KEPT`] windows, window `n` in slot `n % KEPT`.
    cells: [[AtomicU32; 5]; KEPT],
}

impl AudioMeter {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Start or stop measuring. While nobody wants it the audio thread does no work for it.
    pub fn set_wanted(&self, wanted: bool) {
        self.wanted.store(wanted, Ordering::Relaxed);
    }

    pub fn wanted(&self) -> bool {
        self.wanted.load(Ordering::Relaxed)
    }

    /// How many windows were published. It stands still while nothing plays.
    pub fn windows(&self) -> u64 {
        self.windows.load(Ordering::Acquire)
    }

    /// The windows published after the first `seen`, oldest first, and the
    /// count to pass next time. A reader that fell further behind than the
    /// windows kept gets the newest of them.
    pub fn since(&self, seen: u64) -> (u64, Vec<Levels>) {
        let windows = self.windows();
        let first = seen.max(windows.saturating_sub(READABLE));
        let mut levels: Vec<(u64, Levels)> = (first..windows)
            .map(|window| (window, self.read(window)))
            .collect();
        // A window the audio thread overwrote while it was read may be torn: drop it.
        let safe_from = self.windows().saturating_sub(READABLE);
        levels.retain(|(window, _)| *window >= safe_from);
        (
            windows,
            levels.into_iter().map(|(_, levels)| levels).collect(),
        )
    }

    fn read(&self, window: u64) -> Levels {
        let cells = &self.cells[(window % KEPT as u64) as usize];
        let [level, peak, low, mid, high] =
            std::array::from_fn(|index| f32::from_bits(cells[index].load(Ordering::Relaxed)));
        Levels {
            level,
            peak,
            low,
            mid,
            high,
        }
    }

    fn publish(&self, levels: Levels) {
        let window = self.windows.load(Ordering::Relaxed);
        let values = [
            levels.level,
            levels.peak,
            levels.low,
            levels.mid,
            levels.high,
        ];
        let cells = &self.cells[(window % KEPT as u64) as usize];
        for (cell, value) in cells.iter().zip(values) {
            cell.store(value.to_bits(), Ordering::Relaxed);
        }
        self.windows.store(window + 1, Ordering::Release);
    }
}

/// A one-pole low-pass coefficient for `cutoff` at `sample_rate`.
fn coefficient(cutoff: f32, sample_rate: u32) -> f32 {
    1.0 - (-2.0 * std::f32::consts::PI * cutoff / sample_rate.max(1) as f32).exp()
}

/// The audio thread's half: sums a window, then publishes it.
pub(crate) struct MeterTap {
    meter: Arc<AudioMeter>,
    channels: usize,
    window_frames: usize,
    frames: usize,
    squares: [f32; 4],
    peak: f32,
    below_low: f32,
    below_high: f32,
    low_coefficient: f32,
    high_coefficient: f32,
}

impl MeterTap {
    pub(crate) fn new(meter: Arc<AudioMeter>, channels: usize, sample_rate: u32) -> Self {
        Self {
            meter,
            channels: channels.max(1),
            window_frames: ((sample_rate as f32 * WINDOW_SECONDS) as usize).max(1),
            frames: 0,
            squares: [0.0; 4],
            peak: 0.0,
            below_low: 0.0,
            below_high: 0.0,
            low_coefficient: coefficient(LOW_HZ, sample_rate),
            high_coefficient: coefficient(HIGH_HZ, sample_rate),
        }
    }

    /// Fold interleaved `samples` into the window, publishing each one it fills.
    pub(crate) fn measure(&mut self, samples: &[f32]) {
        if !self.meter.wanted() {
            self.frames = 0;
            self.squares = [0.0; 4];
            self.peak = 0.0;
            // The filters' memory is of music long gone by the time someone listens again.
            self.below_low = 0.0;
            self.below_high = 0.0;
            return;
        }
        let channels = self.channels;
        for frame in samples.chunks_exact(channels) {
            let mono = frame.iter().sum::<f32>() / channels as f32;
            let loudest = frame
                .iter()
                .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
            self.peak = self.peak.max(loudest);
            self.below_low += self.low_coefficient * (mono - self.below_low);
            self.below_high += self.high_coefficient * (mono - self.below_high);
            let bands = [
                mono,
                self.below_low,
                self.below_high - self.below_low,
                mono - self.below_high,
            ];
            for (sum, band) in self.squares.iter_mut().zip(bands) {
                *sum += band * band;
            }
            self.frames += 1;
            if self.frames == self.window_frames {
                let rms = |sum: f32| (sum / self.frames as f32).sqrt().min(1.0);
                self.meter.publish(Levels {
                    level: rms(self.squares[0]),
                    peak: self.peak.min(1.0),
                    low: rms(self.squares[1]),
                    mid: rms(self.squares[2]),
                    high: rms(self.squares[3]),
                });
                self.frames = 0;
                self.squares = [0.0; 4];
                self.peak = 0.0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn tone(hz: f32, amplitude: f32, seconds: f32) -> Vec<f32> {
        let frames = (RATE as f32 * seconds) as usize;
        (0..frames)
            .flat_map(|index| {
                let sample = amplitude
                    * (2.0 * std::f32::consts::PI * hz * index as f32 / RATE as f32).sin();
                [sample, sample]
            })
            .collect()
    }

    fn measured(samples: &[f32]) -> (u64, Levels) {
        let meter = AudioMeter::new();
        meter.set_wanted(true);
        let mut tap = MeterTap::new(Arc::clone(&meter), 2, RATE);
        tap.measure(samples);
        let (windows, levels) = meter.since(0);
        (windows, levels.last().copied().unwrap_or_default())
    }

    #[test]
    fn nothing_is_measured_until_someone_asks() {
        let meter = AudioMeter::new();
        let mut tap = MeterTap::new(Arc::clone(&meter), 2, RATE);
        tap.measure(&tone(440.0, 0.5, 0.2));
        assert_eq!(meter.since(0), (0, Vec::new()));
    }

    #[test]
    fn a_reader_gets_every_window_it_missed_up_to_the_ones_kept() {
        let meter = AudioMeter::new();
        meter.set_wanted(true);
        let mut tap = MeterTap::new(Arc::clone(&meter), 2, RATE);
        // Louder each window, so each one can be told apart.
        for step in 1..=3 {
            tap.measure(&tone(440.0, 0.1 * step as f32, WINDOW_SECONDS));
        }
        let (seen, missed) = meter.since(1);
        assert_eq!(seen, 3);
        assert_eq!(missed.len(), 2);
        assert!(missed[0].peak < missed[1].peak, "oldest first: {missed:?}");
        for _ in 0..20 {
            tap.measure(&tone(440.0, 0.5, WINDOW_SECONDS));
        }
        let (seen, behind) = meter.since(seen);
        assert_eq!(seen, 23);
        assert_eq!(
            behind.len(),
            READABLE as usize,
            "only the newest are kept, short of the slot written next"
        );
        assert_eq!(meter.since(seen), (23, Vec::new()));
    }

    #[test]
    fn each_window_is_published_once_it_fills() {
        let (windows, _) = measured(&tone(440.0, 0.5, 0.21));
        assert_eq!(windows, 4);
    }

    #[test]
    fn a_full_scale_tone_reads_as_its_rms_and_peak() {
        let (_, levels) = measured(&tone(440.0, 1.0, 0.1));
        assert!(
            (levels.level - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.02,
            "{levels:?}"
        );
        assert!(levels.peak > 0.99, "{levels:?}");
        let (_, silent) = measured(&vec![0.0; 2 * 4_800]);
        assert_eq!(silent, Levels::default());
    }

    #[test]
    fn a_bass_note_and_a_cymbal_land_in_their_own_bands() {
        let (_, bass) = measured(&tone(60.0, 0.8, 0.5));
        assert!(bass.low > 4.0 * bass.high, "{bass:?}");
        assert!(bass.low > bass.mid, "{bass:?}");
        let (_, cymbal) = measured(&tone(8_000.0, 0.8, 0.5));
        assert!(cymbal.high > 4.0 * cymbal.low, "{cymbal:?}");
        assert!(cymbal.high > cymbal.mid, "{cymbal:?}");
    }
}
