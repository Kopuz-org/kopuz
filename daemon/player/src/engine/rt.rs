//! Real-time audio callback state.
//!
//! The callback *owns* everything it mutates — consumers, fade state, the
//! equalizer's filter memory, channel mode, scratch buffers. Control messages
//! arrive over a lock-free mpsc drained with `try_recv` at block start; retired
//! ring halves are shipped back to the actor so no deallocation happens on the
//! audio thread. The only shared state is atomics: volume, paused, and the
//! per-ring played-sample counter the actor derives position and drain
//! completion from, plus each session's ReplayGain factor.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};

use config::{ChannelMode, EqualizerSettings};

use crate::eq::Equalizer;

/// One playback session as the callback sees it: a ring to drain and the
/// counter that reports how much of it has actually reached the device.
pub(crate) struct RtSession {
    pub consumer: rtrb::Consumer<f32>,
    pub played: Arc<AtomicU64>,
    /// Linear ReplayGain factor for this session's track, as f32 bits. Shared
    /// with the actor, which rewrites it when the settings change, so a live
    /// preamp tweak is heard on the current track without a reload. Per session
    /// because a crossfade has two tracks in flight, each with its own gain.
    pub gain: Arc<AtomicU32>,
    /// An empty ring with this set is the end of the track, not an underrun.
    pub eof: Arc<AtomicBool>,
    /// Raised when a block wanted more than the ring held before the end; the actor clears it.
    pub starved: Arc<AtomicBool>,
    pub applied_gain: GainRamp,
}

/// A step change of gain mid-stream is an audible click.
const GAIN_RAMP_SECONDS: f32 = 0.05;

/// The gain the callback applies, walked towards the session's gain cell.
pub(crate) struct GainRamp {
    current: f32,
    target: f32,
    step: f32,
}

impl GainRamp {
    pub(crate) fn new(gain: f32) -> Self {
        let gain = if gain.is_finite() { gain } else { 1.0 };
        Self {
            current: gain,
            target: gain,
            step: 0.0,
        }
    }

    fn apply(&mut self, samples: &mut [f32], target: f32, channels: usize, ramp_frames: f32) {
        let target = if target.is_finite() { target } else { 1.0 };
        if target != self.target {
            self.target = target;
            self.step = (target - self.current).abs() / ramp_frames.max(1.0);
        }
        if self.current == self.target {
            apply_gain(samples, self.current);
            return;
        }
        for frame in samples.chunks_mut(channels.max(1)) {
            self.current = if self.current < self.target {
                (self.current + self.step).min(self.target)
            } else {
                (self.current - self.step).max(self.target)
            };
            apply_gain(frame, self.current);
        }
    }
}

pub(crate) enum RtCmd {
    /// Install a new active session. `fade: Some((frames, generation))` demotes
    /// the current active session to "fading" and cross-mixes over `frames`;
    /// `None` replaces the active session outright and also drops any fading
    /// session (a seek kills an in-flight crossfade). The generation is echoed
    /// on `FadeComplete` so the actor can drop a completion that raced the
    /// start of a newer fade.
    Swap {
        session: RtSession,
        fade: Option<(u64, u64)>,
    },
    /// Takes over on the same block the active one plays its last sample.
    Queue {
        session: RtSession,
        id: u64,
    },
    /// Drop the queued session `id`. If it already took over, drop the active
    /// session instead: the actor never adopted it, and the outgoing session's
    /// drain then ends the track the usual way.
    Unqueue(u64),
    DropAll,
    SetEqualizer(EqualizerSettings),
    SetChannelMode(ChannelMode),
}

pub(crate) enum Retired {
    /// The consumer is never read again — it rides the message so its ring
    /// buffer deallocates on the actor thread instead of the audio thread.
    Ring(#[allow(dead_code)] rtrb::Consumer<f32>),
    /// The crossfade of this generation ran to completion; the actor reacts by
    /// tearing down the fading worker and emitting `TrackSwitched`.
    FadeComplete(#[allow(dead_code)] rtrb::Consumer<f32>, u64),
    Advanced(#[allow(dead_code)] rtrb::Consumer<f32>, u64),
}

struct Fade {
    total_frames: u64,
    progress_frames: u64,
    generation: u64,
}

/// Frames of scratch capacity; blocks larger than this are processed in chunks
/// so the fade path stays allocation-free regardless of device block size.
const SCRATCH_FRAMES: usize = 4096;

pub(crate) struct RtState {
    cmd_rx: Receiver<RtCmd>,
    retire_tx: Sender<Retired>,
    active: Option<RtSession>,
    fading: Option<RtSession>,
    fade: Option<Fade>,
    next: Option<(RtSession, u64)>,
    /// Kept so a late `Unqueue` can still find a promoted session.
    promoted: Option<u64>,
    eq: Equalizer,
    channel_mode: ChannelMode,
    volume: Arc<AtomicU32>,
    paused: Arc<AtomicBool>,
    scratch_active: Vec<f32>,
    scratch_fading: Vec<f32>,
    channels: usize,
    gain_ramp_frames: f32,
}

pub(crate) fn volume_bits(volume: f32) -> u32 {
    volume.to_bits()
}

impl RtState {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        cmd_rx: Receiver<RtCmd>,
        retire_tx: Sender<Retired>,
        volume: Arc<AtomicU32>,
        paused: Arc<AtomicBool>,
        channels: usize,
        sample_rate: u32,
        eq_settings: EqualizerSettings,
        channel_mode: ChannelMode,
    ) -> Self {
        let channels = channels.max(1);
        let mut eq = Equalizer::new(sample_rate, channels);
        eq.set_settings(eq_settings);
        Self {
            cmd_rx,
            retire_tx,
            active: None,
            fading: None,
            fade: None,
            next: None,
            promoted: None,
            eq,
            channel_mode,
            volume,
            paused,
            scratch_active: vec![0.0; SCRATCH_FRAMES * channels],
            scratch_fading: vec![0.0; SCRATCH_FRAMES * channels],
            channels,
            gain_ramp_frames: sample_rate as f32 * GAIN_RAMP_SECONDS,
        }
    }

    pub(crate) fn process(&mut self, data: &mut [f32]) {
        self.drain_commands();

        if self.paused.load(Ordering::Relaxed) {
            data.fill(0.0);
            return;
        }

        let channels = self.channels;
        let ramp_frames = self.gain_ramp_frames;
        let read = if self.fade.is_some() && self.fading.is_some() {
            self.process_fade(data)
        } else {
            let mut read = self
                .active
                .as_mut()
                .map(|session| read_session(session, data, channels, ramp_frames))
                .unwrap_or(0);
            if read < data.len()
                && self.advance_to_next()
                && let Some(session) = self.active.as_mut()
            {
                read += read_session(session, &mut data[read..], channels, ramp_frames);
            }
            read
        };

        if read < data.len()
            && let Some(active) = &self.active
            && !active.eof.load(Ordering::Acquire)
        {
            active.starved.store(true, Ordering::Relaxed);
        }

        if read > 0 {
            self.eq.process_in_place(&mut data[..read]);
            apply_channel_mode_in_place(&mut data[..read], self.channels, self.channel_mode);

            let volume = f32::from_bits(self.volume.load(Ordering::Relaxed));
            for sample in data[..read].iter_mut() {
                *sample *= volume;
            }
        }
        data[read..].fill(0.0);
    }

    /// The flag is read before the ring, so samples written before the worker's
    /// EOF are never mistaken for the end.
    fn advance_to_next(&mut self) -> bool {
        let drained = self.active.as_ref().is_some_and(|session| {
            session.eof.load(Ordering::Acquire) && session.consumer.is_empty()
        });
        if !drained {
            return false;
        }
        let Some((next, id)) = self.next.take() else {
            return false;
        };
        if let Some(outgoing) = self.active.replace(next) {
            let _ = self
                .retire_tx
                .send(Retired::Advanced(outgoing.consumer, id));
        }
        self.promoted = Some(id);
        true
    }

    fn drop_next(&mut self) {
        if let Some((next, _)) = self.next.take() {
            let _ = self.retire_tx.send(Retired::Ring(next.consumer));
        }
    }

    fn drain_commands(&mut self) {
        while let Ok(cmd) = self.cmd_rx.try_recv() {
            match cmd {
                RtCmd::Swap { session, fade } => {
                    self.drop_next();
                    self.promoted = None;
                    if let Some(old_fading) = self.fading.take() {
                        let _ = self.retire_tx.send(Retired::Ring(old_fading.consumer));
                    }
                    self.fade = None;
                    match (fade, self.active.take()) {
                        (Some((frames, generation)), Some(outgoing)) => {
                            self.fading = Some(outgoing);
                            self.fade = Some(Fade {
                                total_frames: frames.max(1),
                                progress_frames: 0,
                                generation,
                            });
                        }
                        (_, old_active) => {
                            if let Some(old_active) = old_active {
                                let _ = self.retire_tx.send(Retired::Ring(old_active.consumer));
                            }
                        }
                    }
                    self.active = Some(session);
                }
                RtCmd::Queue { session, id } => {
                    self.drop_next();
                    self.next = Some((session, id));
                }
                RtCmd::Unqueue(id) => {
                    if self
                        .next
                        .as_ref()
                        .is_some_and(|(_, next_id)| *next_id == id)
                    {
                        self.drop_next();
                    } else if self.promoted == Some(id) {
                        self.promoted = None;
                        if let Some(active) = self.active.take() {
                            let _ = self.retire_tx.send(Retired::Ring(active.consumer));
                        }
                    }
                }
                RtCmd::DropAll => {
                    self.drop_next();
                    self.promoted = None;
                    if let Some(fading) = self.fading.take() {
                        let _ = self.retire_tx.send(Retired::Ring(fading.consumer));
                    }
                    if let Some(active) = self.active.take() {
                        let _ = self.retire_tx.send(Retired::Ring(active.consumer));
                    }
                    self.fade = None;
                }
                RtCmd::SetEqualizer(settings) => self.eq.set_settings(settings),
                RtCmd::SetChannelMode(mode) => self.channel_mode = mode,
            }
        }
    }

    /// Cross-mix active and fading into `data`, chunked by scratch capacity.
    /// Returns the number of samples written from the start of `data`.
    fn process_fade(&mut self, data: &mut [f32]) -> usize {
        let channels = self.channels;
        let ramp_frames = self.gain_ramp_frames;
        let chunk_capacity = SCRATCH_FRAMES * channels;
        let mut written = 0;
        let mut fade_completed: Option<u64> = None;

        while written < data.len() {
            let chunk_len = (data.len() - written).min(chunk_capacity);
            let chunk = &mut data[written..written + chunk_len];

            let active_scratch = &mut self.scratch_active[..chunk_len];
            let fading_scratch = &mut self.scratch_fading[..chunk_len];
            active_scratch.fill(0.0);
            fading_scratch.fill(0.0);

            let active_read = self
                .active
                .as_mut()
                .map(|s| {
                    let read = read_into(&mut s.consumer, active_scratch);
                    s.played.fetch_add(read as u64, Ordering::Relaxed);
                    apply_session_gain(s, &mut active_scratch[..read], channels, ramp_frames);
                    read
                })
                .unwrap_or(0);
            let fading_read = self
                .fading
                .as_mut()
                .map(|s| {
                    let read = read_into(&mut s.consumer, fading_scratch);
                    s.played.fetch_add(read as u64, Ordering::Relaxed);
                    apply_session_gain(s, &mut fading_scratch[..read], channels, ramp_frames);
                    read
                })
                .unwrap_or(0);

            let read = active_read.max(fading_read);
            if read == 0 {
                break;
            }

            let Some(fade) = self.fade.as_mut() else {
                chunk[..read].copy_from_slice(&active_scratch[..read]);
                written += read;
                break;
            };

            let frames = read / channels;

            let total = fade.total_frames.max(1) as f32;
            let step = 1.0 / total;
            let mut fade_in_gain = fade.progress_frames.min(fade.total_frames) as f32 / total;
            for frame_idx in 0..frames {
                let gain = fade_in_gain.min(1.0);
                let fade_out_gain = 1.0 - gain;
                for ch in 0..channels {
                    let index = frame_idx * channels + ch;
                    chunk[index] =
                        active_scratch[index] * gain + fading_scratch[index] * fade_out_gain;
                }
                fade_in_gain += step;
            }

            chunk[(frames * channels)..read]
                .copy_from_slice(&active_scratch[(frames * channels)..read]);

            fade.progress_frames = fade.progress_frames.saturating_add(frames as u64);
            if fade.progress_frames >= fade.total_frames {
                fade_completed = Some(fade.generation);
            }

            written += read;
            if read < chunk_len {
                break;
            }
        }

        if let Some(generation) = fade_completed {
            self.fade = None;
            if let Some(fading) = self.fading.take() {
                let _ = self
                    .retire_tx
                    .send(Retired::FadeComplete(fading.consumer, generation));
            }
        }

        written
    }
}

fn read_session(
    session: &mut RtSession,
    out: &mut [f32],
    channels: usize,
    ramp_frames: f32,
) -> usize {
    let read = read_into(&mut session.consumer, out);
    session.played.fetch_add(read as u64, Ordering::Relaxed);
    apply_session_gain(session, &mut out[..read], channels, ramp_frames);
    read
}

fn apply_session_gain(
    session: &mut RtSession,
    samples: &mut [f32],
    channels: usize,
    ramp_frames: f32,
) {
    let target = f32::from_bits(session.gain.load(Ordering::Relaxed));
    session
        .applied_gain
        .apply(samples, target, channels, ramp_frames);
}

fn apply_gain(samples: &mut [f32], gain: f32) {
    if !gain.is_finite() || (gain - 1.0).abs() < f32::EPSILON {
        return;
    }
    if gain <= 1.0 {
        for sample in samples.iter_mut() {
            *sample *= gain;
        }
        return;
    }
    for sample in samples.iter_mut() {
        *sample = soft_limit(*sample * gain);
    }
}

const LIMIT_KNEE: f32 = 0.9;

/// Leaves the signal alone below the knee and bends everything above it
/// smoothly towards, never past, full scale.
fn soft_limit(sample: f32) -> f32 {
    let magnitude = sample.abs();
    if magnitude <= LIMIT_KNEE {
        return sample;
    }
    let headroom = 1.0 - LIMIT_KNEE;
    let limited = LIMIT_KNEE + headroom * ((magnitude - LIMIT_KNEE) / headroom).tanh();
    limited.copysign(sample)
}

fn read_into(consumer: &mut rtrb::Consumer<f32>, out: &mut [f32]) -> usize {
    let available = consumer.slots().min(out.len());
    if available == 0 {
        return 0;
    }
    let Ok(chunk) = consumer.read_chunk(available) else {
        return 0;
    };
    let (first, second) = chunk.as_slices();
    out[..first.len()].copy_from_slice(first);
    out[first.len()..first.len() + second.len()].copy_from_slice(second);
    chunk.commit_all();
    available
}

fn apply_channel_mode_to_frame(frame: &mut [f32], mode: ChannelMode) {
    if frame.len() < 2 {
        return;
    }

    let left = frame[0];
    let right = frame[1];

    match mode {
        ChannelMode::Stereo => {}
        ChannelMode::Mono => {
            let mixed = (left + right) * 0.5;
            frame[0] = mixed;
            frame[1] = mixed;
            for sample in &mut frame[2..] {
                *sample = 0.0;
            }
        }
        ChannelMode::LeftOnly => {
            frame[0] = left;
            frame[1] = 0.0;
            for sample in &mut frame[2..] {
                *sample = 0.0;
            }
        }
        ChannelMode::RightOnly => {
            frame[0] = 0.0;
            frame[1] = right;
            for sample in &mut frame[2..] {
                *sample = 0.0;
            }
        }
        ChannelMode::SwapLeftRight => {
            frame[0] = right;
            frame[1] = left;
            for sample in &mut frame[2..] {
                *sample = 0.0;
            }
        }
    }
}

fn apply_channel_mode_in_place(samples: &mut [f32], channels: usize, mode: ChannelMode) {
    if matches!(mode, ChannelMode::Stereo) || channels < 2 {
        return;
    }

    for frame in samples.chunks_exact_mut(channels.max(1)) {
        apply_channel_mode_to_frame(frame, mode);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
    use std::sync::mpsc;

    use config::{ChannelMode, EqualizerSettings};

    use super::{GainRamp, Retired, RtCmd, RtSession, RtState, apply_gain, soft_limit};

    #[test]
    fn gapless_switch_preserves_each_tracks_gain_ramp() {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (retire_tx, retire_rx) = mpsc::channel();
        let mut state = RtState::new(
            cmd_rx,
            retire_tx,
            Arc::new(AtomicU32::new(1.0_f32.to_bits())),
            Arc::new(AtomicBool::new(false)),
            2,
            80,
            EqualizerSettings::default(),
            ChannelMode::Stereo,
        );
        let session = |initial: f32, target: f32| {
            let (mut producer, consumer) = rtrb::RingBuffer::new(8);
            for _ in 0..8 {
                producer.push(1.0).unwrap();
            }
            RtSession {
                consumer,
                played: Arc::new(AtomicU64::new(0)),
                gain: Arc::new(AtomicU32::new(target.to_bits())),
                applied_gain: GainRamp::new(initial),
                eof: Arc::new(AtomicBool::new(true)),
                starved: Arc::new(AtomicBool::new(false)),
            }
        };
        let outgoing = session(1.0, 0.5);
        let incoming = session(0.25, 0.75);
        let outgoing_played = outgoing.played.clone();
        let incoming_played = incoming.played.clone();
        cmd_tx
            .send(RtCmd::Swap {
                session: outgoing,
                fade: None,
            })
            .unwrap();
        cmd_tx
            .send(RtCmd::Queue {
                session: incoming,
                id: 2,
            })
            .unwrap();

        let mut samples = [0.0; 16];
        state.process(&mut samples);

        assert_eq!(
            samples,
            [
                0.875, 0.875, 0.75, 0.75, 0.625, 0.625, 0.5, 0.5, 0.375, 0.375, 0.5, 0.5, 0.625,
                0.625, 0.75, 0.75,
            ]
        );
        assert_eq!(outgoing_played.load(Ordering::Relaxed), 8);
        assert_eq!(incoming_played.load(Ordering::Relaxed), 8);
        assert!(matches!(retire_rx.try_recv(), Ok(Retired::Advanced(_, 2))));
    }

    #[test]
    fn a_ring_that_runs_dry_before_its_end_is_starved() {
        for (eof, starved) in [(false, true), (true, false)] {
            let (cmd_tx, cmd_rx) = mpsc::channel();
            let (retire_tx, _retire_rx) = mpsc::channel();
            let mut state = RtState::new(
                cmd_rx,
                retire_tx,
                Arc::new(AtomicU32::new(1.0_f32.to_bits())),
                Arc::new(AtomicBool::new(false)),
                2,
                80,
                EqualizerSettings::default(),
                ChannelMode::Stereo,
            );
            let (mut producer, consumer) = rtrb::RingBuffer::new(8);
            for _ in 0..4 {
                producer.push(1.0).unwrap();
            }
            let flag = Arc::new(AtomicBool::new(false));
            cmd_tx
                .send(RtCmd::Swap {
                    session: RtSession {
                        consumer,
                        played: Arc::new(AtomicU64::new(0)),
                        gain: Arc::new(AtomicU32::new(1.0_f32.to_bits())),
                        applied_gain: GainRamp::new(1.0),
                        eof: Arc::new(AtomicBool::new(eof)),
                        starved: flag.clone(),
                    },
                    fade: None,
                })
                .unwrap();

            state.process(&mut [0.0; 4]);
            assert!(!flag.load(Ordering::Relaxed), "a full block is no underrun");
            state.process(&mut [0.0; 4]);
            assert_eq!(flag.load(Ordering::Relaxed), starved, "eof {eof}");
        }
    }

    #[test]
    fn a_gain_change_ramps_instead_of_stepping() {
        let mut ramp = GainRamp::new(1.0);
        let mut samples = [1.0_f32; 8];
        ramp.apply(&mut samples, 0.5, 2, 4.0);
        assert_eq!(samples, [0.875, 0.875, 0.75, 0.75, 0.625, 0.625, 0.5, 0.5]);

        let mut settled = [1.0_f32; 4];
        ramp.apply(&mut settled, 0.5, 2, 4.0);
        assert_eq!(settled, [0.5; 4]);
    }

    #[test]
    fn unity_gain_leaves_samples_untouched() {
        let mut ramp = GainRamp::new(1.0);
        let mut samples = [0.25_f32, -0.75, 0.999, -1.0];
        let original = samples;
        ramp.apply(&mut samples, 1.0, 2, 4.0);
        assert_eq!(samples, original);
    }

    #[test]
    fn a_boost_never_pushes_past_full_scale() {
        let mut samples = [0.5, -0.5, 0.95, -0.95, 1.0];
        apply_gain(&mut samples, 4.0);
        assert!(samples.iter().all(|s| s.abs() <= 1.0));
        assert!(samples[0] > 0.9 && samples[1] < -0.9);
    }

    #[test]
    fn the_limiter_leaves_quiet_samples_alone() {
        assert_eq!(soft_limit(0.3), 0.3);
        assert_eq!(soft_limit(-0.9), -0.9);
    }
}
