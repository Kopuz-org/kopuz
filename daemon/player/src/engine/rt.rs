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

use super::mix::MixStart;
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
    /// Samples read in a row under `SILENCE_PEAK`, before gain. The actor polls it.
    pub quiet: Arc<AtomicU64>,
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

/// How a swapped-in session takes over from the active one. The generation
/// is echoed on `FadeComplete` so the actor can drop a completion that raced
/// the start of a newer one.
pub(crate) enum Blend {
    /// Cross-mix with equal-power gains over `frames`.
    Crossfade { frames: u64, generation: u64 },
    /// Hold the new session unread until the outgoing one's `played` reaches
    /// `start`, then sum both at unity until the outgoing one drains. Their
    /// workers already applied the mix's faders, filters and speed.
    Overlay { start: MixStart, generation: u64 },
}

pub(crate) enum RtCmd {
    /// Install a new active session. `fade: Some(blend)` demotes the current
    /// active session to "fading" and blends it out; `None` replaces the
    /// active session outright and also drops any fading session (a seek
    /// kills an in-flight crossfade).
    Swap {
        session: RtSession,
        fade: Option<Blend>,
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

struct Overlay {
    start: MixStart,
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
    overlay: Option<Overlay>,
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
            overlay: None,
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
        } else if self.overlay.is_some() && self.fading.is_some() {
            self.process_overlay(data)
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
                    self.overlay = None;
                    match (fade, self.active.take()) {
                        (Some(Blend::Crossfade { frames, generation }), Some(outgoing)) => {
                            self.fading = Some(outgoing);
                            self.fade = Some(Fade {
                                total_frames: frames.max(1),
                                progress_frames: 0,
                                generation,
                            });
                        }
                        (Some(Blend::Overlay { start, generation }), Some(outgoing)) => {
                            self.fading = Some(outgoing);
                            self.overlay = Some(Overlay { start, generation });
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
                    self.overlay = None;
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
                    track_quiet(s, &active_scratch[..read]);
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
            let mut progress = fade.progress_frames.min(fade.total_frames) as f32 / total;
            for frame_idx in 0..frames {
                let (gain, fade_out_gain) = equal_power(progress);
                for ch in 0..channels {
                    let index = frame_idx * channels + ch;
                    // Equal power lifts in-phase peaks by up to 3 dB; limit the sum.
                    chunk[index] = soft_limit(
                        active_scratch[index] * gain + fading_scratch[index] * fade_out_gain,
                    );
                }
                progress += step;
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

impl RtState {
    /// A planned mix: the outgoing session alone up to the mix start, then
    /// both summed from that very sample, even mid-block. An outgoing session
    /// that drains before the start lets the incoming one in on the next
    /// sample instead of leaving a gap.
    fn process_overlay(&mut self, data: &mut [f32]) -> usize {
        let channels = self.channels;
        let ramp_frames = self.gain_ramp_frames;
        let (Some(overlay), Some(outgoing)) = (self.overlay.as_ref(), self.fading.as_mut()) else {
            return 0;
        };

        let played = outgoing.played.load(Ordering::Relaxed);
        let held = overlay.start.target().saturating_sub(played);
        let held = held.min(data.len() as u64) as usize;
        let written = read_plain(outgoing, &mut data[..held], channels, ramp_frames);
        if written < held && !drained(outgoing) {
            return written;
        }

        let written = written + self.sum_overlay(&mut data[written..]);
        self.retire_overlay();
        written
    }

    /// Outgoing plus incoming into `data`, chunked by scratch capacity.
    fn sum_overlay(&mut self, data: &mut [f32]) -> usize {
        let channels = self.channels;
        let ramp_frames = self.gain_ramp_frames;
        let chunk_capacity = SCRATCH_FRAMES * channels;
        let mut written = 0;

        while written < data.len() {
            let chunk_len = (data.len() - written).min(chunk_capacity);
            let outgoing = &mut self.scratch_fading[..chunk_len];
            let incoming = &mut self.scratch_active[..chunk_len];
            outgoing.fill(0.0);
            incoming.fill(0.0);

            let outgoing_read = self
                .fading
                .as_mut()
                .map_or(0, |s| read_plain(s, outgoing, channels, ramp_frames));
            let incoming_read = self
                .active
                .as_mut()
                .map_or(0, |s| read_session(s, incoming, channels, ramp_frames));
            let read = outgoing_read.max(incoming_read);
            if read == 0 {
                break;
            }

            let chunk = &mut data[written..written + read];
            for ((out, a), b) in chunk.iter_mut().zip(outgoing.iter()).zip(incoming.iter()) {
                *out = a + b;
            }
            written += read;
            if read < chunk_len {
                break;
            }
        }
        written
    }

    /// The outgoing session played its last sample: the mix is over.
    fn retire_overlay(&mut self) {
        if !self.fading.as_ref().is_some_and(drained) {
            return;
        }
        let (Some(overlay), Some(fading)) = (self.overlay.take(), self.fading.take()) else {
            return;
        };
        let _ = self
            .retire_tx
            .send(Retired::FadeComplete(fading.consumer, overlay.generation));
    }
}

/// The worker's end flag is read before the ring, so samples written before
/// it are never mistaken for the end.
fn drained(session: &RtSession) -> bool {
    session.eof.load(Ordering::Acquire) && session.consumer.is_empty()
}

/// Reads a session that does not drive silence detection: the outgoing side
/// of a blend.
fn read_plain(
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

/// Extends or resets the session's run of silent samples. A block counts as
/// silent when its peak stays under the threshold; no allocation, no locking.
fn track_quiet(session: &RtSession, samples: &[f32]) {
    if samples.is_empty() {
        return;
    }
    let peak = samples
        .iter()
        .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
    if peak >= super::SILENCE_PEAK {
        session.quiet.store(0, Ordering::Relaxed);
        return;
    }
    session
        .quiet
        .fetch_add(samples.len() as u64, Ordering::Relaxed);
}

fn read_session(
    session: &mut RtSession,
    out: &mut [f32],
    channels: usize,
    ramp_frames: f32,
) -> usize {
    let read = read_into(&mut session.consumer, out);
    track_quiet(session, &out[..read]);
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

/// Incoming and outgoing gain at `progress` (0..=1) through a crossfade. Their powers sum
/// to one, so two unrelated songs keep their loudness through the middle; a linear fade
/// dips by 3 dB there (0.5 + 0.5 amplitude is half the power).
fn equal_power(progress: f32) -> (f32, f32) {
    let angle = progress.clamp(0.0, 1.0) * std::f32::consts::FRAC_PI_2;
    (angle.sin(), angle.cos())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
    use std::sync::mpsc;

    use config::{ChannelMode, EqualizerSettings};

    use super::{
        Blend, GainRamp, Retired, RtCmd, RtSession, RtState, apply_gain, equal_power, soft_limit,
    };
    use crate::engine::mix::MixStart;

    fn unity_state() -> (mpsc::Sender<RtCmd>, mpsc::Receiver<Retired>, RtState) {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (retire_tx, retire_rx) = mpsc::channel();
        let state = RtState::new(
            cmd_rx,
            retire_tx,
            Arc::new(AtomicU32::new(1.0_f32.to_bits())),
            Arc::new(AtomicBool::new(false)),
            2,
            80,
            EqualizerSettings::default(),
            ChannelMode::Stereo,
        );
        (cmd_tx, retire_rx, state)
    }

    /// A finished ring holding `len` copies of `value`.
    fn filled(value: f32, len: usize) -> RtSession {
        let (mut producer, consumer) = rtrb::RingBuffer::new(len);
        for _ in 0..len {
            producer.push(value).unwrap();
        }
        RtSession {
            consumer,
            played: Arc::new(AtomicU64::new(0)),
            gain: Arc::new(AtomicU32::new(1.0_f32.to_bits())),
            applied_gain: GainRamp::new(1.0),
            eof: Arc::new(AtomicBool::new(true)),
            starved: Arc::new(AtomicBool::new(false)),
            quiet: Arc::new(AtomicU64::new(0)),
        }
    }

    fn overlay(cmd_tx: &mpsc::Sender<RtCmd>, outgoing: RtSession, incoming: RtSession) -> MixStart {
        let start = MixStart::pending();
        cmd_tx
            .send(RtCmd::Swap {
                session: outgoing,
                fade: None,
            })
            .unwrap();
        cmd_tx
            .send(RtCmd::Swap {
                session: incoming,
                fade: Some(Blend::Overlay {
                    start: start.clone(),
                    generation: 7,
                }),
            })
            .unwrap();
        start
    }

    #[test]
    fn an_overlay_starts_the_incoming_deck_mid_block_on_the_mix_start() {
        let (cmd_tx, retire_rx, mut state) = unity_state();
        let start = overlay(&cmd_tx, filled(1.0, 8), filled(0.5, 8));
        start.set(4);

        let mut samples = [0.0; 16];
        state.process(&mut samples);

        assert_eq!(
            samples,
            [
                1.0, 1.0, 1.0, 1.0, 1.5, 1.5, 1.5, 1.5, 0.5, 0.5, 0.5, 0.5, 0.0, 0.0, 0.0, 0.0,
            ]
        );
        assert!(matches!(
            retire_rx.try_recv(),
            Ok(Retired::FadeComplete(_, 7))
        ));
    }

    #[test]
    fn an_overlay_holds_the_incoming_deck_while_the_start_is_pending() {
        let (cmd_tx, _retire_rx, mut state) = unity_state();
        let outgoing = filled(1.0, 16);
        outgoing.eof.store(false, Ordering::Relaxed);
        let incoming = filled(0.5, 8);
        let incoming_played = incoming.played.clone();
        overlay(&cmd_tx, outgoing, incoming);

        let mut samples = [0.0; 8];
        state.process(&mut samples);

        assert_eq!(samples, [1.0; 8]);
        assert_eq!(incoming_played.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn an_outgoing_deck_that_ends_early_lets_the_incoming_one_in() {
        let (cmd_tx, retire_rx, mut state) = unity_state();
        overlay(&cmd_tx, filled(1.0, 4), filled(0.5, 4));

        let mut samples = [0.0; 10];
        state.process(&mut samples);

        assert_eq!(samples, [1.0, 1.0, 1.0, 1.0, 0.5, 0.5, 0.5, 0.5, 0.0, 0.0]);
        assert!(matches!(
            retire_rx.try_recv(),
            Ok(Retired::FadeComplete(_, 7))
        ));
    }

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
                quiet: Arc::new(AtomicU64::new(0)),
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
                        quiet: Arc::new(AtomicU64::new(0)),
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

    #[test]
    fn an_equal_power_fade_keeps_the_power_through_the_middle() {
        for progress in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let (incoming, outgoing) = equal_power(progress);
            assert!((incoming * incoming + outgoing * outgoing - 1.0).abs() < 1e-6);
        }
        assert_eq!(equal_power(0.0), (0.0, 1.0));
        let (incoming, outgoing) = equal_power(1.0);
        assert!((incoming - 1.0).abs() < 1e-6 && outgoing.abs() < 1e-6);
    }

    #[test]
    fn a_crossfade_of_two_loud_tracks_stays_within_full_scale() {
        const FRAMES: usize = 100;
        let loud = || {
            let (mut producer, consumer) = rtrb::RingBuffer::new(FRAMES * 2);
            for _ in 0..FRAMES * 2 {
                producer.push(0.95).unwrap();
            }
            RtSession {
                consumer,
                played: Arc::new(AtomicU64::new(0)),
                gain: Arc::new(AtomicU32::new(1.0_f32.to_bits())),
                applied_gain: GainRamp::new(1.0),
                eof: Arc::new(AtomicBool::new(true)),
                starved: Arc::new(AtomicBool::new(false)),
                quiet: Arc::new(AtomicU64::new(0)),
            }
        };
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (retire_tx, _retire_rx) = mpsc::channel();
        let mut state = RtState::new(
            cmd_rx,
            retire_tx,
            Arc::new(AtomicU32::new(1.0_f32.to_bits())),
            Arc::new(AtomicBool::new(false)),
            2,
            44_100,
            EqualizerSettings::default(),
            ChannelMode::Stereo,
        );
        cmd_tx
            .send(RtCmd::Swap {
                session: loud(),
                fade: None,
            })
            .unwrap();
        cmd_tx
            .send(RtCmd::Swap {
                session: loud(),
                fade: Some(Blend::Crossfade {
                    frames: FRAMES as u64,
                    generation: 1,
                }),
            })
            .unwrap();

        let mut out = [0.0; FRAMES * 2];
        state.process(&mut out);

        let peak = out.iter().fold(0.0_f32, |peak, s| peak.max(s.abs()));
        assert!(peak <= 1.0, "peak {peak}");
    }
}
