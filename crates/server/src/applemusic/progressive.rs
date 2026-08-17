//! Random-access decryption of an Apple Music track, over bytes that are still
//! arriving.
//!
//! A track is thousands of small samples (~800 bytes each) and the CDM costs
//! ~0.2ms apiece, so decrypting everything up front is around a second of dead
//! air before the first note. But samples are independent — each carries its own
//! IV — and CENC is size-preserving, so a sample can be decrypted on its own, in
//! place, in any order.
//!
//! yap yap sahur

use std::io::{Error as IoError, ErrorKind, Read, Result as IoResult, Seek, SeekFrom};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use utils::stream_buffer::BufferProgressCallback;

use super::cenc::{self, Fmp4Layout};
use super::widevine::{Cdm, CdmSession};

/// How often the background filler reports progress, in bytes. The seek bar
/// only needs coarse ranges, and every report crosses a channel.
const PROGRESS_STEP: usize = 64 * 1024;

/// How far ahead of the playhead the filler works before going idle.
const LOOKAHEAD_BYTES: usize = 4 * 1024 * 1024;

/// Samples decrypted between yields during that burst. At ~0.2ms each this hands
/// the lock back every few milliseconds — far inside the 2s ring — so playback
/// still gets served while the filler is busy.
const FILL_BATCH: usize = 32;

const PREBUFFER_BYTES: usize = 256 * 1024;

/// The tail of the file never blocks a read.
const PROBE_TAIL_BYTES: usize = 8 * 1024;

/// Poll interval while waiting on bytes, matching `StreamBuffer`'s sync side.
const WAIT_POLL: Duration = Duration::from_millis(5);

/// How long a read waits for the download to reach it before giving up. A stalled
/// connection should surface as a playback error, not a wedged decoder thread.
const WAIT_TIMEOUT: Duration = Duration::from_secs(30);

/// The file plus everything needed to fill in the rest of it.
///
/// One lock covers the buffer, the sample index, the per-sample flags and the CDM
/// together. That is deliberate: it makes "decrypt this sample unless someone
/// already did" atomic. Decrypting twice would run ciphertext through the cipher a
/// second time and corrupt it, so exactly-once matters more than the fraction of a
/// millisecond the lock is held. Readers and the background thread contend for
/// single samples, never for the whole track.
struct State {
    /// The whole file: ciphertext with decrypted samples written over in place,
    /// sized up front and zero-padded past `downloaded`.
    buf: Vec<u8>,
    /// Bytes received. Everything at or past this is padding, not data.
    downloaded: usize,
    /// The HTTP body finished arriving (successfully or not).
    complete: bool,
    /// Sample index over the downloaded prefix; grows as fragments land.
    layout: Fmp4Layout,
    /// The `downloaded` value `layout` was built from, so a re-walk can be
    /// skipped when nothing new has arrived.
    indexed_at: usize,
    /// Sample-entry boxes relabelled `enca` -> `mp4a`, remembered so the relabel
    /// can be undone around each re-walk. See [`State::reindex`].
    enca_positions: Vec<usize>,
    /// Whether the relabel is currently applied to `buf`.
    patched: bool,
    /// Per sample, in `layout.samples` order.
    decrypted: Vec<bool>,
    key_id: Vec<u8>,
    /// `None` before the licence is loaded, and again once the track is fully
    /// decrypted or for a cache hit.
    cdm: Option<Cdm>,
    /// The CDM session holding this track's content keys. Dropped together with
    /// `cdm` — releasing it any earlier would take the keys with it, mid-track.
    session: Option<CdmSession>,
    error: Option<String>,
}

impl State {
    /// The highest offset that can be served right now.
    ///
    /// Everything up to the end of the last indexed sample is either cleartext
    /// framing or a sample we hold an IV for. Past that the bytes may have
    /// arrived, but we don't yet know how to decrypt them, so they can't be read.
    fn frontier(&self) -> usize {
        self.layout
            .samples
            .last()
            .map(|s| s.end())
            .unwrap_or(self.layout.init_end)
    }

    /// Re-walk the downloaded prefix, extending the sample index.
    ///
    /// Parsing is deterministic and fragments are independent, so a longer prefix
    /// yields the same samples plus more — which is what lets `decrypted` simply
    /// grow alongside it.
    fn reindex(&mut self) {
        if self.indexed_at == self.downloaded {
            return;
        }
        self.indexed_at = self.downloaded;

        // The relabel has to come off first. `patch_init` rewrites the sample
        // entry's type from `enca` to `mp4a` so the decoder accepts the track, but
        // that is the very box the fragment walk looks for to know the track is
        // encrypted — leave it patched and every later walk finds an unencrypted
        // file and indexes nothing. Both directions are a 4-byte write, and the
        // single lock means no reader can observe the buffer mid-flip.
        let restore = std::mem::take(&mut self.enca_positions);
        for &pos in &restore {
            if pos + 8 <= self.buf.len() {
                self.buf[pos + 4..pos + 8].copy_from_slice(b"enca");
            }
        }
        let indexed = cenc::index_fmp4(&self.buf[..self.downloaded]);
        self.enca_positions = restore;

        // An `Err` just means no `moov` yet — too early to index anything.
        if let Ok(fresh) = indexed {
            if fresh.samples.len() >= self.layout.samples.len() {
                if self.enca_positions.is_empty() {
                    self.enca_positions = fresh.enca_positions.clone();
                }
                self.decrypted.resize(fresh.samples.len(), false);
                self.layout = fresh;
            } else {
                // Can't happen with a growing prefix; keeping the longer index is
                // the safe response, since `decrypted` indexes into it.
                tracing::warn!("am.decrypt: re-index shrank the sample list, ignoring it");
            }
        }

        // Put the relabel back (or apply it for the first time) so a read of the
        // init segment sees a track the decoder will open.
        if !self.enca_positions.is_empty() {
            for &pos in &self.enca_positions {
                if pos + 8 <= self.buf.len() {
                    self.buf[pos + 4..pos + 8].copy_from_slice(b"mp4a");
                }
            }
            self.patched = true;
        }
    }
}

/// Feeds a [`ProgressiveTrack`] as the download arrives.
pub struct ChunkSink {
    state: Arc<Mutex<State>>,
}

impl ChunkSink {
    /// Append one chunk of the response body.
    ///
    /// A memcpy under a short lock and nothing else. Re-walking the fragment index
    /// is deliberately left to whoever needs it — the decode thread — because this
    /// runs on a tokio worker, and parsing megabytes there stalls every other
    /// future on that worker, the licence request among them.
    pub fn push(&self, chunk: &[u8]) {
        let Ok(mut s) = self.state.lock() else { return };
        let at = s.downloaded;
        let end = (at + chunk.len()).min(s.buf.len());
        if end > at {
            s.buf[at..end].copy_from_slice(&chunk[..end - at]);
            s.downloaded = end;
        }
    }

    /// The body finished. A final re-walk picks up the last fragment.
    pub fn finish(&self) {
        let Ok(mut s) = self.state.lock() else { return };
        s.reindex();
        s.complete = true;
        if s.downloaded < s.buf.len() {
            // Short read: the rest of the buffer is padding that never arrived.
            let missing = s.buf.len() - s.downloaded;
            tracing::warn!("am.stream: download ended {missing} bytes short");
        }
    }

    pub fn fail(&self, message: String) {
        let Ok(mut s) = self.state.lock() else { return };
        if s.error.is_none() {
            s.error = Some(message);
        }
        s.complete = true;
    }
}

pub struct ProgressiveTrack {
    state: Arc<Mutex<State>>,
    /// Where the decoder has read to, so the filler knows how far ahead it is.
    read_pos: Arc<AtomicUsize>,
    total: u64,
    pos: u64,
}

impl ProgressiveTrack {
    /// A track whose bytes are still downloading.
    ///
    /// Returns the reader plus the sink the download feeds. Nothing can be
    /// decrypted until [`begin_decrypt`](Self::begin_decrypt) supplies the CDM,
    /// so the licence exchange can run while the body is arriving.
    pub fn streaming(total: u64) -> (Self, ChunkSink) {
        let state = Arc::new(Mutex::new(State {
            buf: vec![0u8; total as usize],
            downloaded: 0,
            complete: false,
            layout: Fmp4Layout::default(),
            indexed_at: 0,
            enca_positions: Vec::new(),
            patched: false,
            decrypted: Vec::new(),
            key_id: Vec::new(),
            cdm: None,
            session: None,
            error: None,
        }));
        let track = Self {
            state: state.clone(),
            read_pos: Arc::new(AtomicUsize::new(0)),
            total,
            pos: 0,
        };
        (track, ChunkSink { state })
    }

    /// An already-decrypted track (a cache hit): every read is a memcpy.
    pub fn ready(bytes: Vec<u8>) -> Self {
        let total = bytes.len() as u64;
        let downloaded = bytes.len();
        Self {
            state: Arc::new(Mutex::new(State {
                buf: bytes,
                downloaded,
                complete: true,
                layout: Fmp4Layout::default(),
                indexed_at: downloaded,
                enca_positions: Vec::new(),
                patched: true,
                decrypted: Vec::new(),
                key_id: Vec::new(),
                cdm: None,
                session: None,
                error: None,
            })),
            read_pos: Arc::new(AtomicUsize::new(0)),
            total,
            pos: 0,
        }
    }

    pub fn total_size(&self) -> u64 {
        self.total
    }

    /// Install the content keys and start decrypting.
    ///
    /// Waits for the init segment and the prebuffer to arrive, decrypts that much
    /// so playback doesn't start against an empty ring, then leaves a background
    /// thread to keep ahead of the playhead. `on_complete` gets the finished bytes
    /// once every sample is decrypted, for the disk cache.
    pub fn begin_decrypt(
        &self,
        cdm: Cdm,
        session: CdmSession,
        key_id: Vec<u8>,
        progress: Option<BufferProgressCallback>,
        on_complete: impl FnOnce(Vec<u8>) + Send + 'static,
    ) -> Result<(), String> {
        {
            let mut s = self
                .state
                .lock()
                .map_err(|_| "decrypt state poisoned".to_string())?;
            s.cdm = Some(cdm);
            s.session = Some(session);
            s.key_id = key_id;
        }

        // Build the cushion before the decoder ever reads. Costs a fraction of a
        // second and is what keeps playback from stuttering out of the gate.
        let started = Instant::now();
        let prebuffer_end = {
            let init_end = self.wait_for_init()?;
            (init_end + PREBUFFER_BYTES).min(self.total as usize)
        };
        self.wait_for(prebuffer_end)
            .map_err(|e| format!("prebuffer: {e}"))?;
        self.ensure_range(0, prebuffer_end)
            .map_err(|e| format!("prebuffer: {e}"))?;
        tracing::info!(
            "am.decrypt: prebuffered {} KiB in {:.2}s",
            PREBUFFER_BYTES / 1024,
            started.elapsed().as_secs_f64()
        );
        // The seek bar draws these, same as the HTTP-backed sources. A listener
        // reads it the same way: how much is ready to play.
        if let Some(p) = &progress {
            p(0, prebuffer_end as u64, Some(self.total));
        }

        // Fill in the rest in order, so sequential playback stays ahead of the
        // playhead. A plain thread rather than a task: the decryption itself is
        // CPU-bound, and between bursts this parks waiting on the playhead — so
        // it lives as long as the track does, mostly idle.
        let state = self.state.clone();
        let read_pos = self.read_pos.clone();
        let total = self.total;
        std::thread::Builder::new()
            .name("am-decrypt".into())
            .spawn(move || fill(state, read_pos, total, prebuffer_end, progress, on_complete))
            .map_err(|e| format!("spawn decrypt worker: {e}"))?;
        Ok(())
    }

    /// Wait for the init segment, whose length sets where the prebuffer ends.
    fn wait_for_init(&self) -> Result<usize, String> {
        let deadline = Instant::now() + WAIT_TIMEOUT;
        loop {
            {
                let mut s = self
                    .state
                    .lock()
                    .map_err(|_| "decrypt state poisoned".to_string())?;
                if let Some(e) = &s.error {
                    return Err(e.clone());
                }
                s.reindex();
                if s.layout.init_end > 0 {
                    return Ok(s.layout.init_end);
                }
                if s.complete {
                    return Err("no moov in the downloaded track".to_string());
                }
            }
            if Instant::now() >= deadline {
                return Err("timed out waiting for the init segment".to_string());
            }
            std::thread::sleep(WAIT_POLL);
        }
    }

    /// Block until `want` bytes are indexed, the download ends, or it stalls.
    fn wait_for(&self, want: usize) -> IoResult<()> {
        let deadline = Instant::now() + WAIT_TIMEOUT;
        loop {
            {
                let mut s = self
                    .state
                    .lock()
                    .map_err(|_| IoError::other("decrypt state poisoned"))?;
                if let Some(e) = &s.error {
                    return Err(IoError::other(e.clone()));
                }
                if s.frontier() >= want {
                    return Ok(());
                }
                s.reindex();
                if s.frontier() >= want {
                    return Ok(());
                }
                // Nothing more is coming; the caller reads what there is.
                if s.complete {
                    return Ok(());
                }
            }
            if Instant::now() >= deadline {
                return Err(IoError::new(
                    ErrorKind::TimedOut,
                    "the download stalled before reaching the bytes being read",
                ));
            }
            std::thread::sleep(WAIT_POLL);
        }
    }

    /// Decrypt whatever `[start, end)` overlaps that isn't cleartext yet.
    ///
    /// Bytes outside any sample — the init segment, `moof` headers, box headers —
    /// are already cleartext and cost nothing.
    fn ensure_range(&self, start: usize, end: usize) -> IoResult<()> {
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
        // No CDM means either nothing to decrypt (cache hit) or already finished.
        if s.cdm.is_none() {
            return Ok(());
        }

        let State {
            buf,
            layout,
            decrypted,
            key_id,
            cdm,
            error,
            ..
        } = &mut *s;
        let samples = &layout.samples;
        if samples.is_empty() {
            return Ok(());
        }
        // First sample that could overlap: samples are sorted and disjoint.
        let first = samples.partition_point(|s| s.end() <= start);
        if first >= samples.len() || samples[first].start >= end {
            return Ok(());
        }
        let cdm = cdm.as_ref().expect("checked above");

        let mut i = first;
        while i < samples.len() && samples[i].start < end {
            if !decrypted[i] {
                if let Err(e) = cenc::decrypt_sample(buf, &samples[i], cdm, key_id) {
                    *error = Some(e.clone());
                    return Err(IoError::other(e));
                }
                decrypted[i] = true;
            }
            i += 1;
        }
        Ok(())
    }
}

/// Decrypt every sample in order, waiting for fragments that haven't arrived.
fn fill(
    state: Arc<Mutex<State>>,
    read_pos: Arc<AtomicUsize>,
    total: u64,
    prebuffer_end: usize,
    progress: Option<BufferProgressCallback>,
    on_complete: impl FnOnce(Vec<u8>),
) {
    let mut reported = prebuffer_end;
    let mut index = 0usize;
    let mut stalled_since: Option<Instant> = None;

    loop {
        // What's the next sample, and is there one yet?
        let next = {
            let Ok(mut s) = state.lock() else { return };
            if s.error.is_some() {
                return;
            }
            if index >= s.layout.samples.len() {
                s.reindex();
            }
            match s.layout.samples.get(index) {
                Some(sample) => Some((sample.start, sample.end())),
                None if s.complete => None,
                None => {
                    // More fragments are still coming.
                    drop(s);
                    match stalled_since {
                        Some(since) if since.elapsed() > WAIT_TIMEOUT => {
                            tracing::warn!("am.decrypt: gave up waiting for more fragments");
                            return;
                        }
                        Some(_) => {}
                        None => stalled_since = Some(Instant::now()),
                    }
                    std::thread::sleep(WAIT_POLL);
                    continue;
                }
            }
        };
        let Some((sample_start, sample_end)) = next else {
            break;
        };
        stalled_since = None;

        // Stay a bounded distance ahead of the playhead, then idle. Racing to the
        // end of the track only buys contention.
        while sample_start > read_pos.load(Ordering::Relaxed) + LOOKAHEAD_BYTES {
            std::thread::sleep(Duration::from_millis(100));
        }
        if index.is_multiple_of(FILL_BATCH) {
            // Hand the lock back so a read waiting to copy bytes it already has
            // isn't stuck behind the whole burst.
            std::thread::sleep(Duration::from_millis(1));
        }
        if let Err(e) = ensure_decrypted(&state, index) {
            tracing::warn!("am.decrypt: background fill stopped: {e}");
            return;
        }
        if let Some(p) = &progress
            && sample_end >= reported + PROGRESS_STEP
        {
            p(reported as u64, sample_end as u64, Some(total));
            reported = sample_end;
        }
        index += 1;
    }

    if let Some(p) = &progress
        && reported < total as usize
    {
        p(reported as u64, total, Some(total));
    }

    let finished = {
        let Ok(mut s) = state.lock() else { return };
        // Dropping the handle frees nothing exclusive — it only marks this track
        // done so later reads skip the CDM. The session goes with it: every sample
        // is plaintext now, so nothing needs its keys any more.
        s.cdm = None;
        s.session = None;
        let whole = s.downloaded == s.buf.len();
        (s.error.is_none() && whole).then(|| s.buf.clone())
    };
    tracing::info!("am.decrypt: {index} samples decrypted");
    if let Some(bytes) = finished {
        on_complete(bytes);
    }
}

/// Decrypt sample `index` unless it already is. Idempotent and exactly-once.
fn ensure_decrypted(state: &Mutex<State>, index: usize) -> Result<(), String> {
    let mut s = state
        .lock()
        .map_err(|_| "decrypt state poisoned".to_string())?;

    if let Some(e) = &s.error {
        return Err(e.clone());
    }
    if s.decrypted.get(index).copied().unwrap_or(true) {
        return Ok(());
    }
    // Already finished (CDM released) — nothing left that could need decrypting.
    if s.cdm.is_none() {
        return Ok(());
    }

    // Split the borrow so the buffer, index and CDM can be used together.
    let State {
        buf,
        layout,
        key_id,
        cdm,
        ..
    } = &mut *s;
    let Some(sample) = layout.samples.get(index) else {
        return Ok(());
    };
    let cdm = cdm.as_ref().expect("checked above");
    match cenc::decrypt_sample(buf, sample, cdm, key_id) {
        Ok(()) => {
            s.decrypted[index] = true;
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

        // The probe's end-of-file read is served from padding rather than waited
        // on — see `PROBE_TAIL_BYTES`.
        let tail_begins = (self.total as usize).saturating_sub(PROBE_TAIL_BYTES);
        if start < tail_begins {
            self.wait_for(end.min(tail_begins))?;
        }
        self.ensure_range(start, end)?;

        let s = self
            .state
            .lock()
            .map_err(|_| IoError::other("decrypt state poisoned"))?;
        let n = end - start;
        out[..n].copy_from_slice(&s.buf[start..end]);
        drop(s);
        self.pos += n as u64;
        self.read_pos.store(self.pos as usize, Ordering::Relaxed);
        Ok(n)
    }
}

impl Seek for ProgressiveTrack {
    /// Free: nothing is decrypted or waited on until something is read.
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

    /// A chunk sink fills the buffer from the front and never past the end, so a
    /// server that sends more than it promised can't overflow it.
    #[test]
    fn the_sink_fills_forward_and_is_bounded() {
        let (track, sink) = ProgressiveTrack::streaming(10);
        sink.push(&[1, 2, 3]);
        sink.push(&[4, 5]);
        {
            let s = track.state.lock().unwrap();
            assert_eq!(s.downloaded, 5);
            assert_eq!(&s.buf[..5], &[1, 2, 3, 4, 5]);
            assert_eq!(&s.buf[5..], &[0; 5], "the rest is padding");
        }
        // Overshooting the promised length is clamped, not panicked on.
        sink.push(&[9; 100]);
        let s = track.state.lock().unwrap();
        assert_eq!(s.downloaded, 10);
        assert_eq!(s.buf.len(), 10);
    }

    /// Without this the probe's end-of-file read would block until the whole file
    /// had downloaded, which is exactly the wait streaming exists to remove.
    #[test]
    fn a_read_in_the_probe_tail_does_not_wait() {
        let total = 64 * 1024;
        let (mut track, _sink) = ProgressiveTrack::streaming(total as u64);
        // Nothing downloaded at all, and no `complete` flag to release a waiter.
        track.seek(SeekFrom::End(-160)).unwrap();
        let mut buf = [0u8; 160];
        let started = Instant::now();
        let n = track
            .read(&mut buf)
            .expect("tail reads are served from padding");
        assert_eq!(n, 160);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "tail read waited {:?}",
            started.elapsed()
        );
    }

    /// A read below the tail carve-out with nothing downloaded must not be served
    /// from padding — silently returning zeros would be decoded as audio.
    #[test]
    fn a_read_below_the_tail_waits_and_then_reports_the_stall() {
        let total = 1024 * 1024;
        let (mut track, sink) = ProgressiveTrack::streaming(total as u64);
        // Fail the download so the wait ends deterministically instead of after
        // the 30s timeout.
        sink.fail("connection reset".to_string());
        let mut buf = [0u8; 1024];
        let err = track
            .read(&mut buf)
            .expect_err("a failed download must error");
        assert!(err.to_string().contains("connection reset"), "{err}");
    }

    /// Turn a cached track back into something that indexes like the ciphertext it
    /// came from.
    ///
    /// `patch_init` rewrites the audio sample entry's 4-byte type from `enca` to
    /// `mp4a` and nothing else, so putting it back restores the `sinf`/`tenc` boxes
    /// the fragment walk looks for. The sample *data* stays cleartext, which
    /// indexing doesn't care about — it reads offsets, sizes and IVs.
    ///
    /// Located structurally: `frma` holds the literal `mp4a` too, so searching for
    /// it finds the wrong four bytes. An `stsd` body is
    /// `[version+flags:4][entry_count:4]` then sample entry boxes, putting the
    /// first entry's type at a fixed offset from `stsd`.
    fn restore_enca(bytes: &mut [u8]) -> bool {
        let Some(stsd) = bytes.windows(4).position(|w| w == b"stsd") else {
            return false;
        };
        let entry_type = stsd + 16;
        if entry_type + 4 > bytes.len() || &bytes[entry_type..entry_type + 4] != b"mp4a" {
            return false;
        }
        bytes[entry_type..entry_type + 4].copy_from_slice(b"enca");
        true
    }

    /// Feeds a real fMP4 through the sink in chunks — the only way to exercise the
    /// incremental re-walk against genuine box structure without a live download.
    ///
    /// This is the test that caught `patch_init` eating the `enca` box the walk
    /// needs: the first re-index lands before the first fragment is complete, so it
    /// finds the init segment and no samples, and relabelling there froze the index
    /// for the rest of the track.
    ///
    /// Ignored: needs a cached Apple Music track. No CDM is involved.
    #[test]
    #[ignore = "needs a cached Apple Music track"]
    fn the_index_grows_monotonically_as_chunks_arrive() {
        let dir = std::path::PathBuf::from(std::env::var("HOME").unwrap())
            .join(".cache/kopuz/applemusic");
        let Some(path) = std::fs::read_dir(&dir).ok().and_then(|d| {
            d.flatten()
                .map(|e| e.path())
                .find(|p| p.extension().is_some_and(|e| e == "m4a"))
        }) else {
            return;
        };
        let mut bytes = std::fs::read(&path).expect("read track");
        if !restore_enca(&mut bytes) {
            return;
        }
        let total = bytes.len();

        let (track, sink) = ProgressiveTrack::streaming(total as u64);
        let mut last_frontier = 0usize;
        let mut last_samples = 0usize;
        for chunk in bytes.chunks(64 * 1024) {
            sink.push(chunk);
            let mut s = track.state.lock().unwrap();
            s.reindex();
            let (frontier, samples, downloaded) =
                (s.frontier(), s.layout.samples.len(), s.downloaded);
            assert!(
                frontier >= last_frontier,
                "frontier went backwards: {last_frontier} -> {frontier}"
            );
            assert!(
                samples >= last_samples,
                "sample count shrank: {last_samples} -> {samples}"
            );
            assert!(
                frontier <= downloaded,
                "indexed to {frontier}, only {downloaded} downloaded"
            );
            last_frontier = frontier;
            last_samples = samples;
        }
        sink.finish();

        let s = track.state.lock().unwrap();
        assert_eq!(s.downloaded, total);
        assert!(s.patched, "the init segment should end up relabelled");
        assert!(last_samples > 0, "no samples were ever indexed");
        // The frontier lands inside the final fragment, so allow its framing but
        // nothing like a whole fragment's worth.
        let shortfall = total - s.frontier();
        assert!(
            shortfall < 64 * 1024,
            "index stopped {shortfall} bytes short with {} samples",
            s.layout.samples.len()
        );
    }

    /// `finish` releases readers even when the body arrived short, so a truncated
    /// download surfaces as a decode error rather than a hang.
    #[test]
    fn finishing_short_releases_waiters() {
        let (mut track, sink) = ProgressiveTrack::streaming(1024 * 1024);
        sink.push(&[0u8; 1024]);
        sink.finish();
        let mut buf = [0u8; 4096];
        let started = Instant::now();
        assert!(track.read(&mut buf).is_ok());
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
