//! An Apple Music track decoded to PCM, for platforms whose DRM never hands back
//! the decrypted AAC.
//!
//! On Android the content keys only work inside `MediaCodec`, which decrypts
//! and decodes in one step and returns PCM. So rather than an fMP4 the player
//! is given a WAV: a header computed up front from the track's length, then
//! samples filled in by the decoder as the download arrives.
//!
//! Platform-neutral on purpose. The codec sits behind [`AacDecoder`] and the
//! download behind [`SampleSource`], so everything here runs in desktop tests.

use std::io::{Error as IoError, ErrorKind, Read, Result as IoResult, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::stream::stream_buffer::BufferProgressCallback;

use super::cenc::EncryptedSample;
use super::progressive::ProgressiveTrack;

const HEADER_LEN: u64 = 44;

/// PCM frames per AAC-LC access unit.
pub const AAC_FRAME_SAMPLES: u64 = 1024;

const BYTES_PER_SAMPLE: u16 = 2;

/// How much audio must be decoded before playback is handed the track.
const PREBUFFER: Duration = Duration::from_millis(750);

/// How far past the playhead decoding runs before it waits for the listener.
/// About 45 seconds of 44.1 kHz stereo: decoding is many times faster than real
/// time, so this only spares the battery when a track is skipped.
const LOOKAHEAD_BYTES: u64 = 8 * 1024 * 1024;

const PROGRESS_STEP: u64 = 256 * 1024;

/// How long a read waits on the decoder before reporting a stall.
const WAIT_TIMEOUT: Duration = Duration::from_secs(30);

/// The shape of the PCM a decoder produces. Always signed 16-bit, interleaved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcmFormat {
    pub sample_rate: u32,
    pub channels: u16,
}

impl PcmFormat {
    fn frame_bytes(self) -> u64 {
        u64::from(self.channels) * u64::from(BYTES_PER_SAMPLE)
    }

    /// Bytes of PCM in `duration`, rounded to whole frames.
    fn bytes_for(self, duration: Duration) -> u64 {
        let frames = (duration.as_secs_f64() * f64::from(self.sample_rate)).round() as u64;
        frames * self.frame_bytes()
    }
}

/// A codec that takes encrypted AAC access units and returns PCM.
pub trait AacDecoder: Send {
    /// Queue one access unit. Returns whatever PCM the codec had ready, which
    /// may be nothing yet: codecs hold a frame or two back.
    fn decode(
        &mut self,
        sample: &EncryptedSample,
        data: &[u8],
        pts_us: u64,
    ) -> Result<Vec<u8>, String>;

    /// End the stream and collect what is still inside the codec.
    fn finish(&mut self) -> Result<Vec<u8>, String>;

    /// The PCM layout, once the codec has reported it.
    fn format(&self) -> Option<PcmFormat>;
}

/// Where encrypted samples come from, in order.
pub trait SampleSource: Send {
    /// Sample `index`, waiting for it to arrive; `None` past the last one.
    fn sample(&self, index: usize) -> Result<Option<(EncryptedSample, Vec<u8>)>, String>;
}

impl SampleSource for ProgressiveTrack {
    fn sample(&self, index: usize) -> Result<Option<(EncryptedSample, Vec<u8>)>, String> {
        self.encrypted_sample(index)
    }
}

#[derive(Default)]
struct Decoded {
    pcm: Vec<u8>,
    format: Option<PcmFormat>,
    /// Length of the WAV, header included, fixed once the format is known.
    total: Option<u64>,
    done: bool,
    error: Option<String>,
}

struct Shared {
    decoded: Mutex<Decoded>,
    changed: Condvar,
    /// Where the reader is in the WAV, so decoding knows how far ahead it is.
    read_pos: AtomicU64,
    /// Set when the track is dropped, so the decode thread stops with it.
    cancelled: AtomicBool,
}

impl Shared {
    fn lock(&self) -> IoResult<std::sync::MutexGuard<'_, Decoded>> {
        self.decoded
            .lock()
            .map_err(|_| IoError::other("PCM state poisoned"))
    }
}

pub struct PcmTrack {
    shared: Arc<Shared>,
    header: [u8; HEADER_LEN as usize],
    total: u64,
    pos: u64,
}

impl PcmTrack {
    /// Start decoding `source` and return once the first stretch of audio is in.
    ///
    /// `input_rate` is the AAC track's sample rate, for timestamps. `duration`
    /// sets the WAV's length; if the decoder produces less, the rest reads as
    /// silence, and anything past it is cut off.
    pub fn start<S, D>(
        source: S,
        decoder: D,
        input_rate: u32,
        duration: Duration,
        progress: Option<BufferProgressCallback>,
    ) -> Result<Self, String>
    where
        S: SampleSource + 'static,
        D: AacDecoder + 'static,
    {
        let shared = Arc::new(Shared {
            decoded: Mutex::new(Decoded::default()),
            changed: Condvar::new(),
            read_pos: AtomicU64::new(0),
            cancelled: AtomicBool::new(false),
        });

        let worker = shared.clone();
        std::thread::Builder::new()
            .name("am-pcm-decode".into())
            .spawn(move || decode_all(source, decoder, input_rate, duration, &worker, progress))
            .map_err(|e| format!("spawn the decode thread: {e}"))?;

        let started = Instant::now();
        let deadline = started + WAIT_TIMEOUT;
        let mut decoded = shared
            .decoded
            .lock()
            .map_err(|_| "PCM state poisoned".to_string())?;
        let (format, total) = loop {
            if let (Some(format), Some(total)) = (decoded.format, decoded.total) {
                let have = decoded.pcm.len() as u64;
                if have >= format.bytes_for(PREBUFFER) || (decoded.done && have > 0) {
                    break (format, total);
                }
            }
            if let Some(e) = &decoded.error {
                return Err(e.clone());
            }
            if decoded.done {
                return Err("the decoder produced no audio".to_string());
            }
            let now = Instant::now();
            if now >= deadline {
                return Err("timed out waiting for the first decoded audio".to_string());
            }
            decoded = shared
                .changed
                .wait_timeout(decoded, deadline - now)
                .map_err(|_| "PCM state poisoned".to_string())?
                .0;
        };
        drop(decoded);

        tracing::info!(
            rate = format.sample_rate,
            channels = format.channels,
            "am.pcm: prebuffered in {:.2}s",
            started.elapsed().as_secs_f64()
        );

        Ok(Self {
            shared,
            header: wav_header(format, total - HEADER_LEN),
            total,
            pos: 0,
        })
    }

    pub fn total_size(&self) -> u64 {
        self.total
    }
}

impl Drop for PcmTrack {
    fn drop(&mut self) {
        self.shared.cancelled.store(true, Ordering::Relaxed);
        self.shared.changed.notify_all();
    }
}

/// Run the whole track through `decoder`, appending PCM as it comes out.
fn decode_all<S: SampleSource, D: AacDecoder>(
    source: S,
    mut decoder: D,
    input_rate: u32,
    duration: Duration,
    shared: &Shared,
    progress: Option<BufferProgressCallback>,
) {
    let mut reported = 0u64;
    let mut append = |pcm: Vec<u8>, format: Option<PcmFormat>| {
        let Ok(mut d) = shared.decoded.lock() else {
            return;
        };
        if d.format.is_none()
            && let Some(format) = format
        {
            d.format = Some(format);
            d.total = Some(HEADER_LEN + format.bytes_for(duration));
        }
        d.pcm.extend_from_slice(&pcm);
        let end = HEADER_LEN + d.pcm.len() as u64;
        if let (Some(p), Some(total)) = (&progress, d.total)
            && end >= reported + PROGRESS_STEP
        {
            p(reported, end.min(total), Some(total));
            reported = end;
        }
        drop(d);
        shared.changed.notify_all();
    };

    let mut index = 0usize;
    let outcome = loop {
        if shared.cancelled.load(Ordering::Relaxed) {
            return;
        }
        while HEADER_LEN + decoded_len(shared)
            > shared.read_pos.load(Ordering::Relaxed) + LOOKAHEAD_BYTES
        {
            if shared.cancelled.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        match source.sample(index) {
            Ok(Some((sample, data))) => {
                let pts_us =
                    index as u64 * AAC_FRAME_SAMPLES * 1_000_000 / u64::from(input_rate.max(1));
                match decoder.decode(&sample, &data, pts_us) {
                    Ok(pcm) => append(pcm, decoder.format()),
                    Err(e) => break Err(e),
                }
            }
            Ok(None) => match decoder.finish() {
                Ok(pcm) => {
                    append(pcm, decoder.format());
                    break Ok(());
                }
                Err(e) => break Err(e),
            },
            Err(e) => break Err(e),
        }
        index += 1;
    };

    if let Ok(mut d) = shared.decoded.lock() {
        d.done = true;
        match outcome {
            Ok(()) => {
                tracing::info!("am.pcm: decoded {index} samples, {} bytes", d.pcm.len());
                if let (Some(p), Some(total)) = (&progress, d.total)
                    && reported < total
                {
                    p(reported, total, Some(total));
                }
            }
            Err(e) => {
                tracing::warn!("am.pcm: decoding stopped: {e}");
                d.error = Some(e);
            }
        }
    }
    shared.changed.notify_all();
}

fn decoded_len(shared: &Shared) -> u64 {
    shared
        .decoded
        .lock()
        .map(|d| d.pcm.len() as u64)
        .unwrap_or(0)
}

/// A canonical 44-byte header for 16-bit PCM.
fn wav_header(format: PcmFormat, data_len: u64) -> [u8; HEADER_LEN as usize] {
    let data_len = u32::try_from(data_len).unwrap_or(u32::MAX - 36);
    let block_align = format.channels * BYTES_PER_SAMPLE;
    let byte_rate = format.sample_rate * u32::from(block_align);

    let mut h = [0u8; HEADER_LEN as usize];
    h[0..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&(36 + data_len).to_le_bytes());
    h[8..12].copy_from_slice(b"WAVE");
    h[12..16].copy_from_slice(b"fmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&1u16.to_le_bytes());
    h[22..24].copy_from_slice(&format.channels.to_le_bytes());
    h[24..28].copy_from_slice(&format.sample_rate.to_le_bytes());
    h[28..32].copy_from_slice(&byte_rate.to_le_bytes());
    h[32..34].copy_from_slice(&block_align.to_le_bytes());
    h[34..36].copy_from_slice(&(BYTES_PER_SAMPLE * 8).to_le_bytes());
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&data_len.to_le_bytes());
    h
}

impl Read for PcmTrack {
    fn read(&mut self, out: &mut [u8]) -> IoResult<usize> {
        if out.is_empty() || self.pos >= self.total {
            return Ok(0);
        }
        let want = (out.len() as u64).min(self.total - self.pos) as usize;

        if self.pos < HEADER_LEN {
            let start = self.pos as usize;
            let n = want.min(HEADER_LEN as usize - start);
            out[..n].copy_from_slice(&self.header[start..start + n]);
            self.pos += n as u64;
            return Ok(n);
        }

        self.shared.read_pos.store(self.pos, Ordering::Relaxed);
        let offset = (self.pos - HEADER_LEN) as usize;
        let deadline = Instant::now() + WAIT_TIMEOUT;
        let mut decoded = self.shared.lock()?;
        loop {
            if decoded.pcm.len() > offset {
                break;
            }
            if let Some(e) = &decoded.error {
                return Err(IoError::other(e.clone()));
            }
            if decoded.done {
                break;
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(IoError::new(
                    ErrorKind::TimedOut,
                    "decoding stalled before reaching the audio being read",
                ));
            }
            decoded = self
                .shared
                .changed
                .wait_timeout(decoded, deadline - now)
                .map_err(|_| IoError::other("PCM state poisoned"))?
                .0;
        }

        let n = if decoded.pcm.len() > offset {
            let n = want.min(decoded.pcm.len() - offset);
            out[..n].copy_from_slice(&decoded.pcm[offset..offset + n]);
            n
        } else {
            out[..want].fill(0);
            want
        };
        drop(decoded);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for PcmTrack {
    /// Free: the reader waits for the decoder only when it reads.
    fn seek(&mut self, from: SeekFrom) -> IoResult<u64> {
        let target = match from {
            SeekFrom::Start(n) => n as i64,
            SeekFrom::Current(d) => self.pos as i64 + d,
            SeekFrom::End(d) => self.total as i64 + d,
        };
        if target < 0 {
            return Err(IoError::new(
                ErrorKind::InvalidInput,
                "seek before start of track",
            ));
        }
        self.pos = (target as u64).min(self.total);
        self.shared.read_pos.store(self.pos, Ordering::Relaxed);
        Ok(self.pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FORMAT: PcmFormat = PcmFormat {
        sample_rate: 1000,
        channels: 1,
    };

    /// Samples whose bytes are their own index, so the PCM shows which came out.
    struct Samples(usize);

    impl SampleSource for Samples {
        fn sample(&self, index: usize) -> Result<Option<(EncryptedSample, Vec<u8>)>, String> {
            Ok((index < self.0).then(|| {
                let sample = EncryptedSample {
                    start: 0,
                    len: 1,
                    iv: [0; 16],
                    subs: Vec::new(),
                };
                (sample, vec![index as u8])
            }))
        }
    }

    /// Turns each access unit into 1000 bytes (500 mono frames, half a second at
    /// 1 kHz), holding the newest one back the way real codecs hold a frame.
    #[derive(Default)]
    struct Echo {
        held: Option<u8>,
        fail_at: Option<u8>,
    }

    impl AacDecoder for Echo {
        fn decode(&mut self, _: &EncryptedSample, data: &[u8], _: u64) -> Result<Vec<u8>, String> {
            if Some(data[0]) == self.fail_at {
                return Err("bad sample".into());
            }
            let out = self.held.map(|b| vec![b; 1000]).unwrap_or_default();
            self.held = Some(data[0]);
            Ok(out)
        }

        fn finish(&mut self) -> Result<Vec<u8>, String> {
            Ok(self.held.take().map(|b| vec![b; 1000]).unwrap_or_default())
        }

        fn format(&self) -> Option<PcmFormat> {
            Some(FORMAT)
        }
    }

    fn read_all(track: &mut PcmTrack) -> Vec<u8> {
        let mut out = Vec::new();
        track.read_to_end(&mut out).expect("read");
        out
    }

    #[test]
    fn the_header_describes_the_declared_length() {
        let mut track = PcmTrack::start(
            Samples(4),
            Echo::default(),
            1000,
            Duration::from_secs(2),
            None,
        )
        .expect("start");
        assert_eq!(track.total_size(), HEADER_LEN + 4000);
        let bytes = read_all(&mut track);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 4000);
        assert_eq!(u32::from_le_bytes(bytes[24..28].try_into().unwrap()), 1000);
    }

    #[test]
    fn the_audio_comes_out_in_order_including_what_the_codec_held_back() {
        let mut track = PcmTrack::start(
            Samples(4),
            Echo::default(),
            1000,
            Duration::from_secs(2),
            None,
        )
        .expect("start");
        let pcm = read_all(&mut track).split_off(HEADER_LEN as usize);
        assert_eq!(pcm.len(), 4000);
        for (i, chunk) in pcm.chunks(1000).enumerate() {
            assert!(chunk.iter().all(|&b| b == i as u8), "chunk {i}");
        }
    }

    /// A length read from the container can disagree with what the codec
    /// produces. Short reads as silence rather than an early end, and long is
    /// cut at the declared length, so the header always stays truthful.
    #[test]
    fn a_short_decode_is_padded_and_a_long_one_cut() {
        let mut short = PcmTrack::start(
            Samples(2),
            Echo::default(),
            1000,
            Duration::from_secs(2),
            None,
        )
        .expect("start");
        let pcm = read_all(&mut short).split_off(HEADER_LEN as usize);
        assert_eq!(pcm.len(), 4000);
        assert!(pcm[2000..].iter().all(|&b| b == 0));

        let mut long = PcmTrack::start(
            Samples(6),
            Echo::default(),
            1000,
            Duration::from_secs(1),
            None,
        )
        .expect("start");
        let pcm = read_all(&mut long).split_off(HEADER_LEN as usize);
        assert_eq!(pcm.len(), 2000);
    }

    #[test]
    fn seeking_reads_from_the_new_place() {
        let mut track = PcmTrack::start(
            Samples(4),
            Echo::default(),
            1000,
            Duration::from_secs(2),
            None,
        )
        .expect("start");
        track
            .seek(SeekFrom::Start(HEADER_LEN + 2500))
            .expect("seek");
        let mut buf = [0u8; 10];
        track.read_exact(&mut buf).expect("read");
        assert_eq!(buf, [2; 10]);
    }

    #[test]
    fn a_decoder_error_reaches_the_reader() {
        let decoder = Echo {
            fail_at: Some(3),
            ..Echo::default()
        };
        let mut track = PcmTrack::start(Samples(4), decoder, 1000, Duration::from_secs(2), None)
            .expect("start");
        let mut out = Vec::new();
        let error = track.read_to_end(&mut out).expect_err("the error surfaces");
        assert!(error.to_string().contains("bad sample"));
    }

    #[test]
    fn a_track_with_no_audio_fails_to_start() {
        let result = PcmTrack::start(
            Samples(0),
            Echo::default(),
            1000,
            Duration::from_secs(1),
            None,
        );
        assert!(result.is_err());
    }
}
