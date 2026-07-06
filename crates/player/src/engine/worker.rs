//! Per-session decode worker.
//!
//! One thread per loaded source. It builds the media source (the factory may
//! block on network), probes it, then decodes packets into the ring buffer.
//! All coordination is over channels — no shared flags. At natural EOF the
//! worker parks on its command channel with the format reader alive, so a
//! later `Seek` resumes in place instead of stranding the session (the
//! seek-after-end hang this refactor removes).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::Duration;

use symphonia::core::audio::GenericAudioBufferRef;
use symphonia::core::codecs::audio::{AudioCodecParameters, AudioDecoder, AudioDecoderOptions};
use symphonia::core::codecs::registry::RegisterableAudioDecoder;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, Track};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::Time;

use super::SourceFactory;

pub(crate) enum WorkerCmd {
    /// Begin decoding into the given ring. Sent once, after `Ready`.
    Start {
        producer: rtrb::Producer<f32>,
        written: Arc<AtomicU64>,
        channels: usize,
        sample_rate: u32,
        start_at: Option<Duration>,
    },
    /// Re-seek and switch to a fresh ring (the old one was swapped out of the
    /// audio callback; pre-seek samples die with it).
    Seek {
        target: Duration,
        producer: rtrb::Producer<f32>,
        written: Arc<AtomicU64>,
    },
    Stop,
}

pub(crate) enum WorkerMsg {
    /// Probe finished; the actor picks an output config and replies `Start`.
    Ready {
        token: u64,
        source_sample_rate: Option<u32>,
        seekable: bool,
    },
    /// Natural end of the source. The worker stays parked and seekable.
    Eof { token: u64 },
    /// Source construction / probe / codec setup failed before playback.
    Failed { token: u64, error: String },
}

pub(crate) struct WorkerHandle {
    pub cmd_tx: Sender<WorkerCmd>,
    pub join: std::thread::JoinHandle<()>,
}

pub(crate) fn spawn<M, F>(
    token: u64,
    factory: SourceFactory,
    msg_tx: Sender<M>,
    wrap: F,
) -> WorkerHandle
where
    M: Send + 'static,
    F: Fn(WorkerMsg) -> M + Send + 'static,
{
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel();
    let join = std::thread::Builder::new()
        .name(format!("kopuz-decode-{token}"))
        .spawn(move || run(token, factory, &msg_tx, &wrap, &cmd_rx))
        .expect("failed to spawn decode worker thread");
    WorkerHandle { cmd_tx, join }
}

struct Output {
    producer: rtrb::Producer<f32>,
    written: Arc<AtomicU64>,
}

enum FlowChange {
    /// A seek arrived: pre-seek output must be discarded.
    Seeked,
    /// Stop requested or the actor went away.
    Exit,
}

fn run<M, F>(
    token: u64,
    factory: SourceFactory,
    msg_tx: &Sender<M>,
    wrap: &F,
    cmd_rx: &Receiver<WorkerCmd>,
) where
    F: Fn(WorkerMsg) -> M,
{
    let send = |msg: WorkerMsg| {
        let _ = msg_tx.send(wrap(msg));
    };
    let fail = |error: String| {
        tracing::error!(token, error = %error, "decode worker failed");
        send(WorkerMsg::Failed { token, error });
    };

    let (source, hint) = match factory() {
        Ok(parts) => parts,
        Err(e) => return fail(format!("failed to open source: {e}")),
    };
    let seekable = source.is_seekable();

    let mss = MediaSourceStream::new(source, Default::default());
    let mut format: Box<dyn FormatReader> = match symphonia::default::get_probe().probe(
        &hint,
        mss,
        FormatOptions::default(),
        MetadataOptions::default(),
    ) {
        Ok(format) => format,
        Err(e) => return fail(format!("symphonia probe error: {e}")),
    };

    let Some(track) = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.as_ref().and_then(|p| p.audio()).is_some())
    else {
        return fail("no supported audio tracks found".to_string());
    };
    let track_id = track.id;
    // YouTube Music WebM/Opus streams reach the codec layer with channels
    // empty — symphonia's matroska demuxer doesn't always propagate it, and
    // both the built-in Opus decoder and the libopus adapter then bail with
    // "channels required." Parse OpusHead from extra_data, or fall back to
    // stereo at 48 kHz.
    let Some(audio_params) = audio_params_for_track(track) else {
        return fail("no audio codec parameters".to_string());
    };
    let source_sample_rate = audio_params.sample_rate;

    let mut decoder: Box<dyn AudioDecoder> = match symphonia::default::get_codecs()
        .make_audio_decoder(&audio_params, &AudioDecoderOptions::default())
    {
        Ok(d) => d,
        Err(_) => match symphonia_adapter_libopus::OpusDecoder::try_registry_new(
            &audio_params,
            &AudioDecoderOptions::default(),
        ) {
            Ok(d) => d,
            Err(e) => return fail(format!("symphonia codec error: {e}")),
        },
    };

    send(WorkerMsg::Ready {
        token,
        source_sample_rate,
        seekable,
    });

    // Wait for the actor's decision. A superseded load simply drops our
    // command sender, which lands here as an error → exit.
    let (mut output, target_channels, target_sample_rate) = match cmd_rx.recv() {
        Ok(WorkerCmd::Start {
            producer,
            written,
            channels,
            sample_rate,
            start_at,
        }) => {
            if let Some(target) = start_at {
                seek_reader(format.as_mut(), decoder.as_mut(), track_id, target);
            }
            (Output { producer, written }, channels, sample_rate)
        }
        _ => return,
    };

    let source_sample_rate = source_sample_rate.unwrap_or(target_sample_rate);

    loop {
        match drain_commands(
            cmd_rx,
            &mut output,
            format.as_mut(),
            decoder.as_mut(),
            track_id,
        ) {
            None => {}
            Some(FlowChange::Seeked) => continue,
            Some(FlowChange::Exit) => return,
        }

        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => {
                send(WorkerMsg::Eof { token });
                match park_at_eof(
                    cmd_rx,
                    &mut output,
                    format.as_mut(),
                    decoder.as_mut(),
                    track_id,
                ) {
                    ParkOutcome::Resume => continue,
                    ParkOutcome::Exit => return,
                }
            }
            Err(symphonia::core::errors::Error::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                send(WorkerMsg::Eof { token });
                match park_at_eof(
                    cmd_rx,
                    &mut output,
                    format.as_mut(),
                    decoder.as_mut(),
                    track_id,
                ) {
                    ParkOutcome::Resume => continue,
                    ParkOutcome::Exit => return,
                }
            }
            Err(symphonia::core::errors::Error::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(e) => {
                tracing::warn!(error = %e, "format error — ending track");
                send(WorkerMsg::Eof { token });
                match park_at_eof(
                    cmd_rx,
                    &mut output,
                    format.as_mut(),
                    decoder.as_mut(),
                    track_id,
                ) {
                    ParkOutcome::Resume => continue,
                    ParkOutcome::Exit => return,
                }
            }
        };

        if packet.track_id != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(symphonia::core::errors::Error::DecodeError(e)) => {
                tracing::debug!(error = %e, "recoverable decode error — skipping packet");
                continue;
            }
            Err(e) => {
                tracing::error!(error = %e, "fatal decode error");
                send(WorkerMsg::Eof { token });
                match park_at_eof(
                    cmd_rx,
                    &mut output,
                    format.as_mut(),
                    decoder.as_mut(),
                    track_id,
                ) {
                    ParkOutcome::Resume => continue,
                    ParkOutcome::Exit => return,
                }
            }
        };

        let samples = audio_buf_to_f32_interleaved(
            &decoded,
            target_channels,
            source_sample_rate,
            target_sample_rate,
        );

        match write_all(
            cmd_rx,
            &mut output,
            &samples,
            format.as_mut(),
            decoder.as_mut(),
            track_id,
        ) {
            None => {}
            Some(FlowChange::Seeked) => continue,
            Some(FlowChange::Exit) => return,
        }
    }
}

enum ParkOutcome {
    Resume,
    Exit,
}

/// Block on the command channel with the reader alive so a later seek can
/// resume this session in place.
fn park_at_eof(
    cmd_rx: &Receiver<WorkerCmd>,
    output: &mut Output,
    format: &mut dyn FormatReader,
    decoder: &mut dyn AudioDecoder,
    track_id: u32,
) -> ParkOutcome {
    loop {
        match cmd_rx.recv() {
            Ok(WorkerCmd::Seek {
                target,
                producer,
                written,
            }) => {
                *output = Output { producer, written };
                seek_reader(format, decoder, track_id, target);
                return ParkOutcome::Resume;
            }
            Ok(WorkerCmd::Stop) | Err(_) => return ParkOutcome::Exit,
            Ok(WorkerCmd::Start { .. }) => {
                tracing::warn!("unexpected Start for an already-started decode worker");
            }
        }
    }
}

/// Apply any pending commands without blocking.
fn drain_commands(
    cmd_rx: &Receiver<WorkerCmd>,
    output: &mut Output,
    format: &mut dyn FormatReader,
    decoder: &mut dyn AudioDecoder,
    track_id: u32,
) -> Option<FlowChange> {
    let mut seeked = false;
    loop {
        match cmd_rx.try_recv() {
            Ok(WorkerCmd::Seek {
                target,
                producer,
                written,
            }) => {
                *output = Output { producer, written };
                seek_reader(format, decoder, track_id, target);
                seeked = true;
            }
            Ok(WorkerCmd::Stop) => return Some(FlowChange::Exit),
            Ok(WorkerCmd::Start { .. }) => {
                tracing::warn!("unexpected Start for an already-started decode worker");
            }
            Err(TryRecvError::Empty) => {
                return if seeked {
                    Some(FlowChange::Seeked)
                } else {
                    None
                };
            }
            Err(TryRecvError::Disconnected) => return Some(FlowChange::Exit),
        }
    }
}

/// Write the full sample block, backing off while the ring is full and
/// staying responsive to Seek/Stop.
fn write_all(
    cmd_rx: &Receiver<WorkerCmd>,
    output: &mut Output,
    samples: &[f32],
    format: &mut dyn FormatReader,
    decoder: &mut dyn AudioDecoder,
    track_id: u32,
) -> Option<FlowChange> {
    let mut offset = 0;
    while offset < samples.len() {
        if let Some(change) = drain_commands(cmd_rx, output, format, decoder, track_id) {
            // On seek the rest of this pre-seek block is garbage — drop it.
            return Some(change);
        }

        let available = output.producer.slots().min(samples.len() - offset);
        if available == 0 {
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }
        let Ok(chunk) = output.producer.write_chunk_uninit(available) else {
            std::thread::sleep(Duration::from_millis(5));
            continue;
        };
        let written = chunk.fill_from_iter(samples[offset..offset + available].iter().copied());
        offset += written;
        output.written.fetch_add(written as u64, Ordering::Relaxed);
    }
    None
}

fn seek_reader(
    format: &mut dyn FormatReader,
    decoder: &mut dyn AudioDecoder,
    track_id: u32,
    target: Duration,
) {
    let time = Time::try_from_secs_f64(target.as_secs_f64()).unwrap_or_default();
    let seek_to = SeekTo::Time {
        time,
        track_id: Some(track_id),
    };
    // Symphonia demuxers can panic on malformed streams mid-seek.
    let seek_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        format.seek(SeekMode::Coarse, seek_to)
    }));
    match seek_result {
        Ok(Ok(_)) => decoder.reset(),
        Ok(Err(e)) => tracing::warn!(error = %e, "seek error"),
        Err(_) => {
            tracing::warn!("seek panicked inside symphonia demuxer; continuing playback");
            decoder.reset();
        }
    }
}

pub(crate) fn parse_opushead_channels(extra: &[u8]) -> Option<u8> {
    if extra.len() >= 10 && &extra[..8] == b"OpusHead" {
        Some(extra[9])
    } else {
        None
    }
}

pub(crate) fn audio_params_for_track(track: &Track) -> Option<AudioCodecParameters> {
    let mut audio_params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .cloned()?;

    if audio_params.channels.is_none() {
        let ch = audio_params
            .extra_data
            .as_deref()
            .and_then(parse_opushead_channels)
            .unwrap_or(2);
        audio_params.channels = Some(symphonia::core::audio::Channels::Discrete(ch as u16));
        if audio_params.sample_rate.is_none() {
            audio_params.sample_rate = Some(48_000);
        }
    }

    Some(audio_params)
}

fn audio_buf_to_f32_interleaved(
    buf: &GenericAudioBufferRef,
    target_channels: usize,
    source_sample_rate: u32,
    target_sample_rate: u32,
) -> Vec<f32> {
    let src_chans = buf.num_planes().max(1);
    let mut interleaved: Vec<f32> = Vec::with_capacity(buf.frames() * src_chans);
    buf.copy_to_vec_interleaved(&mut interleaved);

    let interleaved = if src_chans != target_channels {
        convert_channels(&interleaved, src_chans, target_channels)
    } else {
        interleaved
    };

    if source_sample_rate != target_sample_rate {
        resample(
            &interleaved,
            target_channels,
            source_sample_rate,
            target_sample_rate,
        )
    } else {
        interleaved
    }
}

fn convert_channels(samples: &[f32], src_channels: usize, dst_channels: usize) -> Vec<f32> {
    let frames = samples.len() / src_channels;
    let mut out = Vec::with_capacity(frames * dst_channels);

    for frame in 0..frames {
        let src_offset = frame * src_channels;
        for ch in 0..dst_channels {
            if ch < src_channels {
                out.push(samples[src_offset + ch]);
            } else if src_channels == 1 {
                // Mono to multi: duplicate
                out.push(samples[src_offset]);
            } else {
                out.push(0.0);
            }
        }
    }
    out
}

fn resample(samples: &[f32], channels: usize, src_rate: u32, dst_rate: u32) -> Vec<f32> {
    if channels == 0 || src_rate == 0 || dst_rate == 0 {
        return samples.to_vec();
    }
    let src_frames = samples.len() / channels;
    let ratio = dst_rate as f64 / src_rate as f64;
    if ratio.is_nan() || ratio.is_infinite() {
        return samples.to_vec();
    }
    let dst_frames = (src_frames as f64 * ratio).ceil() as usize;
    let mut out = Vec::with_capacity(dst_frames * channels);

    for i in 0..dst_frames {
        let src_pos = i as f64 / ratio;
        let src_idx = src_pos.floor() as usize;
        let frac = src_pos - src_idx as f64;

        for ch in 0..channels {
            let s0 = if src_idx < src_frames {
                samples[src_idx * channels + ch]
            } else {
                0.0
            };
            let s1 = if src_idx + 1 < src_frames {
                samples[(src_idx + 1) * channels + ch]
            } else {
                s0
            };
            out.push(s0 + (s1 - s0) * frac as f32);
        }
    }
    out
}
