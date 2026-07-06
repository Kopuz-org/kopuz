//! Real-time audio callback state.
//!
//! The callback *owns* everything it mutates — consumers, fade state, the
//! equalizer's filter memory, channel mode, scratch buffers. Control messages
//! arrive over a lock-free mpsc drained with `try_recv` at block start; retired
//! ring halves are shipped back to the actor so no deallocation happens on the
//! audio thread. The only shared state is atomics: volume, paused, and the
//! per-ring played-sample counter the actor derives position and drain
//! completion from.

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
}

pub(crate) enum RtCmd {
    /// Install a new active session. `fade_frames: Some(n)` demotes the current
    /// active session to "fading" and cross-mixes over `n` frames; `None`
    /// replaces the active session outright and also drops any fading session
    /// (a seek kills an in-flight crossfade).
    Swap {
        session: RtSession,
        fade_frames: Option<u64>,
    },
    DropAll,
    SetEqualizer(EqualizerSettings),
    SetChannelMode(ChannelMode),
}

pub(crate) enum Retired {
    /// The consumer is never read again — it rides the message so its ring
    /// buffer deallocates on the actor thread instead of the audio thread.
    Ring(#[allow(dead_code)] rtrb::Consumer<f32>),
    /// The crossfade ran to completion; the actor reacts by tearing down the
    /// fading worker and emitting `TrackSwitched`.
    FadeComplete(#[allow(dead_code)] rtrb::Consumer<f32>),
}

struct Fade {
    total_frames: u64,
    progress_frames: u64,
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
    eq: Equalizer,
    channel_mode: ChannelMode,
    volume: Arc<AtomicU32>,
    paused: Arc<AtomicBool>,
    scratch_active: Vec<f32>,
    scratch_fading: Vec<f32>,
    channels: usize,
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
            eq,
            channel_mode,
            volume,
            paused,
            scratch_active: vec![0.0; SCRATCH_FRAMES * channels],
            scratch_fading: vec![0.0; SCRATCH_FRAMES * channels],
            channels,
        }
    }

    pub(crate) fn process(&mut self, data: &mut [f32]) {
        self.drain_commands();

        if self.paused.load(Ordering::Relaxed) {
            data.fill(0.0);
            return;
        }

        let read = if self.fade.is_some() && self.fading.is_some() {
            self.process_fade(data)
        } else {
            self.active
                .as_mut()
                .map(|session| {
                    let read = read_into(&mut session.consumer, data);
                    session.played.fetch_add(read as u64, Ordering::Relaxed);
                    read
                })
                .unwrap_or(0)
        };

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

    fn drain_commands(&mut self) {
        while let Ok(cmd) = self.cmd_rx.try_recv() {
            match cmd {
                RtCmd::Swap {
                    session,
                    fade_frames,
                } => {
                    if let Some(old_fading) = self.fading.take() {
                        let _ = self.retire_tx.send(Retired::Ring(old_fading.consumer));
                    }
                    self.fade = None;
                    match (fade_frames, self.active.take()) {
                        (Some(frames), Some(outgoing)) => {
                            self.fading = Some(outgoing);
                            self.fade = Some(Fade {
                                total_frames: frames.max(1),
                                progress_frames: 0,
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
                RtCmd::DropAll => {
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
        let chunk_capacity = SCRATCH_FRAMES * channels;
        let mut written = 0;
        let mut fade_completed = false;

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
                    read
                })
                .unwrap_or(0);
            let fading_read = self
                .fading
                .as_mut()
                .map(|s| read_into(&mut s.consumer, fading_scratch))
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
            for frame_idx in 0..frames {
                let progress = ((fade.progress_frames + frame_idx as u64).min(fade.total_frames))
                    as f32
                    / fade.total_frames.max(1) as f32;
                let fade_in_gain = progress.clamp(0.0, 1.0);
                let fade_out_gain = 1.0 - fade_in_gain;
                for ch in 0..channels {
                    let index = frame_idx * channels + ch;
                    chunk[index] = active_scratch[index] * fade_in_gain
                        + fading_scratch[index] * fade_out_gain;
                }
            }
            // A trailing partial frame (read not divisible by channels) is
            // passed through unmixed.
            chunk[(frames * channels)..read]
                .copy_from_slice(&active_scratch[(frames * channels)..read]);

            fade.progress_frames = fade.progress_frames.saturating_add(frames as u64);
            if fade.progress_frames >= fade.total_frames {
                fade_completed = true;
            }

            // Past-total frames mix at saturated gains (1.0 active / 0.0
            // fading), so the block is still filled; teardown happens below.
            written += read;
            if read < chunk_len {
                break;
            }
        }

        if fade_completed {
            self.fade = None;
            if let Some(fading) = self.fading.take() {
                let _ = self.retire_tx.send(Retired::FadeComplete(fading.consumer));
            }
        }

        written
    }
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
