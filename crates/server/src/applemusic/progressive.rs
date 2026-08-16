//! Random-access decryption of an Apple Music track.
//!
//! The CDM costs ~1.6ms per sample, and a track holds thousands of them, so
//! decrypting everything up front costs ~15s before the first note. But samples
//! are independent — each carries its own IV — and CENC is size-preserving, so
//! a sample can be decrypted on its own, in place, in any order.
//!
//! So the file is held as ciphertext with the init segment patched, and each
//! sample is decrypted the moment something actually reads it. A read only pays
//! for the bytes it touches (~800 bytes and ~1.6ms per sample), a seek costs
//! nothing at all, and anything already decrypted is a plain memcpy. A
//! background thread fills in the rest in order so sequential playback stays
//! ahead of the playhead, and a fully decrypted track is cached to disk.

use std::io::{Error as IoError, ErrorKind, Read, Result as IoResult, Seek, SeekFrom};
use std::sync::{Arc, Mutex};

use super::cenc::{self, Fmp4Layout};
use super::widevine::Cdm;

const PREBUFFER_BYTES: usize = 256 * 1024;

/// The file plus everything needed to fill in the rest of it.
///
/// One lock covers the buffer, the per-sample flags and the CDM together. That
/// is deliberate: it makes "decrypt this sample unless someone already did"
/// atomic. Decrypting twice would run ciphertext through the cipher a second
/// time and corrupt it, so exactly-once matters more than the ~1.6ms the lock is
/// held. Readers and the background thread contend for single samples, never for
/// the whole track.
struct State {
    /// Whole file: ciphertext, with decrypted samples written over in place.
    buf: Vec<u8>,
    /// Per sample, in `layout.samples` order.
    decrypted: Vec<bool>,
    /// `None` once the track is fully decrypted, or for a cache hit.
    cdm: Option<Cdm>,
    remaining: usize,
    error: Option<String>,
}

pub struct ProgressiveTrack {
    layout: Arc<Fmp4Layout>,
    key_id: Arc<Vec<u8>>,
    state: Arc<Mutex<State>>,
    total: u64,
    pos: u64,
}

impl ProgressiveTrack {
    /// Index `encrypted`, then decrypt it lazily.
    ///
    /// Returns as soon as the (CDM-free) index is built. `on_complete` gets the
    /// finished bytes once every sample is decrypted, for the disk cache.
    pub fn spawn(
        encrypted: Vec<u8>,
        cdm: Cdm,
        key_id: Vec<u8>,
        on_complete: impl FnOnce(Vec<u8>) + Send + 'static,
    ) -> Result<Self, String> {
        let layout = Arc::new(cenc::index_fmp4(&encrypted)?);
        let total = encrypted.len() as u64;

        let mut buf = encrypted;
        cenc::patch_init(&mut buf, &layout);

        let sample_count = layout.samples.len();
        let state = Arc::new(Mutex::new(State {
            buf,
            decrypted: vec![false; sample_count],
            cdm: Some(cdm),
            remaining: sample_count,
            error: None,
        }));

        let track = Self {
            layout: layout.clone(),
            key_id: Arc::new(key_id),
            state: state.clone(),
            total,
            pos: 0,
        };

        // Build the cushion before the decoder ever reads. Costs a fraction of a
        // second and is what keeps playback from stuttering out of the gate.
        let prebuffer_end = layout.init_end + PREBUFFER_BYTES;
        let started = std::time::Instant::now();
        track
            .ensure_range(0, prebuffer_end)
            .map_err(|e| format!("prebuffer: {e}"))?;
        tracing::info!(
            "am.decrypt: prebuffered {} KiB in {:.2}s",
            PREBUFFER_BYTES / 1024,
            started.elapsed().as_secs_f64()
        );

        // Fill in the rest in order, so sequential playback stays ahead of the
        // playhead. A plain thread: this is CPU-bound and runs for a while.
        let key_id = track.key_id.clone();
        std::thread::Builder::new()
            .name("am-decrypt".into())
            .spawn(move || {
                let started = std::time::Instant::now();
                for index in 0..sample_count {
                    if let Err(e) = ensure_decrypted(&state, &layout, &key_id, index) {
                        tracing::warn!("am.decrypt: background fill stopped: {e}");
                        return;
                    }
                }
                let finished = {
                    let mut s = match state.lock() {
                        Ok(s) => s,
                        Err(_) => return,
                    };
                    // Release the CDM session here rather than at drop: the next
                    // track shouldn't wait on the cache write below.
                    s.cdm = None;
                    s.error.is_none().then(|| s.buf.clone())
                };
                tracing::info!(
                    "am.decrypt: fully decrypted {sample_count} samples in {:.2}s",
                    started.elapsed().as_secs_f64()
                );
                if let Some(bytes) = finished {
                    on_complete(bytes);
                }
            })
            .map_err(|e| format!("spawn decrypt worker: {e}"))?;

        Ok(track)
    }

    /// An already-decrypted track (a cache hit): every read is a memcpy.
    pub fn ready(bytes: Vec<u8>) -> Self {
        let total = bytes.len() as u64;
        Self {
            layout: Arc::new(Fmp4Layout::default()),
            key_id: Arc::new(Vec::new()),
            state: Arc::new(Mutex::new(State {
                buf: bytes,
                decrypted: Vec::new(),
                cdm: None,
                remaining: 0,
                error: None,
            })),
            total,
            pos: 0,
        }
    }

    pub fn total_size(&self) -> u64 {
        self.total
    }

    /// Decrypt whatever `[start, end)` overlaps that isn't cleartext yet.
    ///
    /// Bytes outside any sample — the init segment, `moof` headers, box headers —
    /// are already cleartext and cost nothing.
    fn ensure_range(&self, start: usize, end: usize) -> IoResult<()> {
        let samples = &self.layout.samples;
        if samples.is_empty() {
            return Ok(());
        }
        // First sample that could overlap: samples are sorted and disjoint.
        let first = samples.partition_point(|s| s.end() <= start);
        if first >= samples.len() || samples[first].start >= end {
            return Ok(());
        }

        // One lock for the whole range, not one per sample: re-acquiring it
        // between samples lets the background filler cut in each time, so a read
        // that needs 80 samples ends up interleaved 80 times. A reader is
        // latency-critical (the audio device is draining); the filler is not.
        let mut s = self
            .state
            .lock()
            .map_err(|_| IoError::other("decrypt state poisoned"))?;
        if let Some(e) = &s.error {
            return Err(IoError::other(e.clone()));
        }
        let State {
            buf,
            decrypted,
            cdm,
            remaining,
            error,
        } = &mut *s;
        // No CDM means the track is already fully decrypted.
        let Some(cdm) = cdm.as_ref() else {
            return Ok(());
        };

        let mut i = first;
        while i < samples.len() && samples[i].start < end {
            if !decrypted[i] {
                if let Err(e) = cenc::decrypt_sample(buf, &samples[i], cdm, &self.key_id) {
                    *error = Some(e.clone());
                    return Err(IoError::other(e));
                }
                decrypted[i] = true;
                *remaining -= 1;
            }
            i += 1;
        }
        Ok(())
    }
}

/// Decrypt sample `index` unless it already is. Idempotent and exactly-once.
fn ensure_decrypted(
    state: &Mutex<State>,
    layout: &Fmp4Layout,
    key_id: &[u8],
    index: usize,
) -> Result<(), String> {
    let mut s = state
        .lock()
        .map_err(|_| "decrypt state poisoned".to_string())?;

    if let Some(e) = &s.error {
        return Err(e.clone());
    }
    if s.decrypted.get(index).copied().unwrap_or(true) {
        return Ok(());
    }
    let Some(sample) = layout.samples.get(index) else {
        return Ok(());
    };
    // Already finished (CDM released) — nothing left that could need decrypting.
    if s.cdm.is_none() {
        return Ok(());
    }

    // Split the borrow so the buffer and the CDM can be used together.
    let State { buf, cdm, .. } = &mut *s;
    let cdm = cdm.as_ref().expect("checked above");
    match cenc::decrypt_sample(buf, sample, cdm, key_id) {
        Ok(()) => {
            s.decrypted[index] = true;
            s.remaining -= 1;
            Ok(())
        }
        Err(e) => {
            s.error = Some(e.clone());
            Err(e)
        }
    }
}

impl Read for ProgressiveTrack {
    fn read(&mut self, out: &mut [u8]) -> IoResult<usize> {
        if out.is_empty() || self.pos >= self.total {
            return Ok(0);
        }
        let start = self.pos as usize;
        let end = (start + out.len()).min(self.total as usize);
        self.ensure_range(start, end)?;

        let s = self
            .state
            .lock()
            .map_err(|_| IoError::other("decrypt state poisoned"))?;
        let n = end - start;
        out[..n].copy_from_slice(&s.buf[start..end]);
        drop(s);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for ProgressiveTrack {
    /// Free: nothing is decrypted until something is read.
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
        Ok(self.pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cache_hit_reads_straight_through() {
        let mut t = ProgressiveTrack::ready(vec![5u8; 10]);
        let mut all = Vec::new();
        t.read_to_end(&mut all).unwrap();
        assert_eq!(all, vec![5u8; 10]);
        assert_eq!(t.total_size(), 10);
    }

    #[test]
    fn seeking_is_free_and_absolute() {
        let mut t = ProgressiveTrack::ready((0u8..100).collect());
        assert_eq!(t.seek(SeekFrom::End(-10)).unwrap(), 90);
        assert_eq!(t.seek(SeekFrom::Start(5)).unwrap(), 5);
        assert_eq!(t.seek(SeekFrom::Current(3)).unwrap(), 8);
        // Past the end clamps rather than erroring, matching a file.
        assert_eq!(t.seek(SeekFrom::Start(1_000)).unwrap(), 100);
        assert!(t.seek(SeekFrom::Start(0)).is_ok());
        assert!(t.seek(SeekFrom::Current(-1)).is_err());
    }

    #[test]
    fn reading_after_a_seek_returns_that_region() {
        let mut t = ProgressiveTrack::ready((0u8..100).collect());
        t.seek(SeekFrom::Start(90)).unwrap();
        let mut tail = Vec::new();
        t.read_to_end(&mut tail).unwrap();
        assert_eq!(tail, (90u8..100).collect::<Vec<_>>());
    }

    /// The seam that makes on-demand decryption cheap: a read must map to just
    /// the samples it overlaps, not the whole track.
    #[test]
    fn a_read_touches_only_overlapping_samples() {
        let samples: Vec<cenc::EncryptedSample> = (0..10)
            .map(|i| cenc::EncryptedSample {
                start: 100 + i * 10,
                len: 10,
                iv: [0; 16],
                subs: Vec::new(),
            })
            .collect();

        // Bytes 100..110 are sample 0, 110..120 sample 1, and so on.
        let first = |start: usize| samples.partition_point(|s| s.end() <= start);
        assert_eq!(first(0), 0, "a read before any sample starts at sample 0");
        assert_eq!(first(100), 0);
        assert_eq!(first(109), 0, "mid-sample reads include that sample");
        assert_eq!(first(110), 1);
        assert_eq!(first(195), 9);
        assert_eq!(first(200), 10, "past the last sample selects none");
    }
}
