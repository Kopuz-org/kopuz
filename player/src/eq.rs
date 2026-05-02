use config::EqualizerSettings;

const BAND_FREQUENCIES: [f32; 5] = [60.0, 250.0, 1_000.0, 4_000.0, 12_000.0];
const BAND_Q: [f32; 5] = [0.9, 1.0, 1.0, 0.9, 0.8];

#[derive(Clone, Copy)]
struct Coefficients {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Coefficients {
    const fn identity() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
        }
    }
}

#[derive(Clone)]
struct Biquad {
    coeffs: Coefficients,
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn new(coeffs: Coefficients) -> Self {
        Self {
            coeffs,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn set_coefficients(&mut self, coeffs: Coefficients) {
        self.coeffs = coeffs;
    }

    fn process(&mut self, sample: f32) -> f32 {
        let output = self.coeffs.b0 * sample + self.z1;
        self.z1 = self.coeffs.b1 * sample - self.coeffs.a1 * output + self.z2;
        self.z2 = self.coeffs.b2 * sample - self.coeffs.a2 * output;
        output
    }

    fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

#[derive(Clone)]
struct Band {
    frequency: f32,
    q: f32,
    gain_db: f32,
    filters: Vec<Biquad>,
}

impl Band {
    fn new(channels: usize, frequency: f32, q: f32) -> Self {
        Self {
            frequency,
            q,
            gain_db: 0.0,
            filters: vec![Biquad::new(Coefficients::identity()); channels.max(1)],
        }
    }

    fn ensure_channels(&mut self, channels: usize, sample_rate: u32) {
        let channels = channels.max(1);
        if self.filters.len() == channels {
            return;
        }

        let coeffs = self.current_coefficients(sample_rate);
        self.filters = vec![Biquad::new(coeffs); channels];
    }

    fn update_gain(&mut self, gain_db: f32, sample_rate: u32) {
        self.gain_db = gain_db;
        let coeffs = peaking_coefficients(sample_rate, self.frequency, self.q, gain_db);
        for filter in &mut self.filters {
            filter.set_coefficients(coeffs);
        }
    }

    fn reset_filters(&mut self) {
        for filter in &mut self.filters {
            filter.reset();
        }
    }

    fn process(&mut self, channel: usize, sample: f32) -> f32 {
        let index = if channel < self.filters.len() {
            channel
        } else {
            self.filters.len().saturating_sub(1)
        };
        self.filters[index].process(sample)
    }

    fn current_coefficients(&self, sample_rate: u32) -> Coefficients {
        if self.gain_db.abs() < 0.01 {
            Coefficients::identity()
        } else {
            peaking_coefficients(sample_rate, self.frequency, self.q, self.gain_db)
        }
    }
}

pub struct Equalizer {
    settings: EqualizerSettings,
    sample_rate: u32,
    channels: usize,
    bands: [Band; 5],
    output_gain: f32,
}

impl Equalizer {
    pub fn new(sample_rate: u32, channels: usize) -> Self {
        let mut equalizer = Self {
            settings: EqualizerSettings::default(),
            sample_rate: sample_rate.max(1),
            channels: channels.max(1),
            bands: std::array::from_fn(|index| {
                Band::new(channels.max(1), BAND_FREQUENCIES[index], BAND_Q[index])
            }),
            output_gain: 1.0,
        };
        equalizer.rebuild(false);
        equalizer
    }

    pub fn set_settings(&mut self, settings: EqualizerSettings) {
        self.settings = settings;
        self.rebuild(false);
    }

    pub fn update_output_format(&mut self, sample_rate: u32, channels: usize) {
        self.sample_rate = sample_rate.max(1);
        self.channels = channels.max(1);
        self.rebuild(true);
    }

    pub fn process_in_place(&mut self, samples: &mut [f32]) {
        if !self.settings.enabled {
            return;
        }

        for frame in samples.chunks_exact_mut(self.channels.max(1)) {
            for (channel, sample) in frame.iter_mut().enumerate() {
                let mut value = *sample * self.output_gain;
                for band in &mut self.bands {
                    value = band.process(channel, value);
                }
                *sample = value.clamp(-1.0, 1.0);
            }
        }
    }

    fn rebuild(&mut self, reset_filter_state: bool) {
        let resolved_bands = self.settings.resolved_bands();
        let max_boost = resolved_bands
            .iter()
            .copied()
            .fold(0.0_f32, f32::max)
            .max(0.0);

        self.output_gain = db_to_linear(self.settings.preamp_db - max_boost);

        for (band, gain) in self.bands.iter_mut().zip(resolved_bands) {
            band.ensure_channels(self.channels, self.sample_rate);
            band.update_gain(gain, self.sample_rate);
            if reset_filter_state {
                band.reset_filters();
            }
        }
    }
}

fn peaking_coefficients(sample_rate: u32, frequency: f32, q: f32, gain_db: f32) -> Coefficients {
    if gain_db.abs() < 0.01 || sample_rate == 0 {
        return Coefficients::identity();
    }

    let a = 10.0_f32.powf(gain_db / 40.0);
    let omega = 2.0 * std::f32::consts::PI * frequency / sample_rate as f32;
    let alpha = omega.sin() / (2.0 * q.max(0.001));
    let cos_omega = omega.cos();

    let b0 = 1.0 + alpha * a;
    let b1 = -2.0 * cos_omega;
    let b2 = 1.0 - alpha * a;
    let a0 = 1.0 + alpha / a;
    let a1 = -2.0 * cos_omega;
    let a2 = 1.0 - alpha / a;

    Coefficients {
        b0: b0 / a0,
        b1: b1 / a0,
        b2: b2 / a0,
        a1: a1 / a0,
        a2: a2 / a0,
    }
}

fn db_to_linear(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

#[cfg(test)]
mod tests {
    use super::Equalizer;
    use config::{EqPreset, EqualizerSettings};

    #[test]
    fn disabled_equalizer_leaves_samples_unchanged() {
        let mut equalizer = Equalizer::new(48_000, 2);
        equalizer.set_settings(EqualizerSettings {
            enabled: false,
            ..Default::default()
        });

        let mut samples = vec![0.25, -0.25, 0.1, -0.1];
        let original = samples.clone();
        equalizer.process_in_place(&mut samples);

        assert_eq!(samples, original);
    }

    #[test]
    fn preset_with_boost_changes_non_silent_audio() {
        let mut equalizer = Equalizer::new(48_000, 2);
        equalizer.set_settings(EqualizerSettings {
            enabled: true,
            preset: EqPreset::BassBoost,
            ..Default::default()
        });

        let mut samples = vec![0.2, 0.2, 0.15, 0.15, 0.1, 0.1];
        let original = samples.clone();
        equalizer.process_in_place(&mut samples);

        assert_ne!(samples, original);
    }
}
