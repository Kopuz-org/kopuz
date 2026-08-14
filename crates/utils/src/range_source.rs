//! HTTP Range-backed seekable byte source.
//!
//! Used for remote media servers and YouTube Music when the URL returns
//! `Accept-Ranges: bytes`). Unlike [`crate::stream_buffer::StreamBuffer`],
//! this never downloads the file linearly — every miss in the rolling
//! window cache becomes a `Range: bytes=N-M` request. Symphonia can seek
//! freely: to the end (Matroska Cues), to scrub targets, anywhere.
//!
//! Architecture:
//! - One rolling 512 KiB window, anchored at `chunk_start`.
//! - `seek()` is a constant-time pointer move.
//! - `read()` fetches a fresh window only when `pos` falls outside the
//!   currently-cached window. Sequential playback stays inside the window
//!   90%+ of the time.
//! - HTTP calls happen via `reqwest::blocking` inside whatever thread is
//!   calling `Read::read` — callers MUST already be on a blocking-friendly
//!   thread (`spawn_blocking` or similar).
//!
//! `byte_len()` is determined once upfront from `Content-Range` of the
//! initial probe fetch. If the server ignores ranges or omits the total,
//! this source can't be constructed and the caller can stream sequentially.

use std::io::{Error as IoError, ErrorKind, Read, Result as IoResult, Seek, SeekFrom};
use std::time::Duration;

use crate::stream_buffer::BufferProgressCallback;

const CHUNK: usize = 512 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// An in-flight background fetch of the window at `start`, so sequential
/// playback overlaps the next window's download with decoding the current one
/// instead of stalling the decoder a full network round-trip at every 512 KiB
/// boundary — the mid-song stutter on slow links. A Mutex+Condvar slot rather
/// than an mpsc receiver because the decoder wants the whole source `Sync`.
struct Prefetch {
    start: u64,
    slot: std::sync::Arc<PrefetchSlot>,
}

#[derive(Default)]
struct PrefetchSlot {
    result: std::sync::Mutex<Option<IoResult<Vec<u8>>>>,
    ready: std::sync::Condvar,
}

impl PrefetchSlot {
    fn put(&self, value: IoResult<Vec<u8>>) {
        let mut guard = self.result.lock().unwrap_or_else(|e| e.into_inner());
        *guard = Some(value);
        self.ready.notify_all();
    }

    fn take_blocking(&self) -> IoResult<Vec<u8>> {
        let mut guard = self.result.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(value) = guard.take() {
                return value;
            }
            guard = self.ready.wait(guard).unwrap_or_else(|e| e.into_inner());
        }
    }

    fn try_take(&self) -> Option<IoResult<Vec<u8>>> {
        self.result.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
}

pub struct RangeStreamSource {
    url: String,
    client: reqwest::blocking::Client,
    total_size: u64,
    pos: u64,
    chunk: Vec<u8>,
    chunk_start: u64,
    prefetch: Option<Prefetch>,
    progress: Option<BufferProgressCallback>,
}

impl RangeStreamSource {
    /// Probe the URL with a `Range: bytes=0-0` HEAD-equivalent to learn its
    /// total size and confirm Range support. Returns the source positioned
    /// at byte 0 with an empty cache.
    pub fn new(url: String, user_agent: Option<String>) -> IoResult<Self> {
        Self::new_with_progress(url, user_agent, None)
    }

    pub fn new_with_progress(
        url: String,
        user_agent: Option<String>,
        progress: Option<BufferProgressCallback>,
    ) -> IoResult<Self> {
        let ua =
            user_agent.unwrap_or_else(|| concat!("Kopuz/", env!("CARGO_PKG_VERSION")).to_string());
        let client = shared_client(&ua)?;

        // One-byte probe — cheap, and the server returns the full
        // `Content-Range: bytes 0-0/<TOTAL>` we want.
        let resp = client
            .get(&url)
            .header("Range", "bytes=0-0")
            .send()
            // `without_url`: a stream URL can carry credentials in its userinfo,
            // and a reqwest error prints the URL it failed on.
            .map_err(|e| IoError::other(e.without_url()))?;
        let status = resp.status();
        if status != reqwest::StatusCode::PARTIAL_CONTENT {
            return Err(IoError::new(
                ErrorKind::Unsupported,
                format!("server ignored range probe (HTTP {status})"),
            ));
        }
        let total_size = parse_total_size(&resp)
            .ok_or_else(|| IoError::other("range response didn't expose total size"))?;

        Ok(Self {
            url,
            client,
            total_size,
            pos: 0,
            chunk: Vec::with_capacity(CHUNK),
            chunk_start: 0,
            prefetch: None,
            progress,
        })
    }

    pub fn total_size(&self) -> u64 {
        self.total_size
    }

    fn install_chunk(&mut self, start: u64, bytes: Vec<u8>) {
        let end = start + bytes.len() as u64;
        self.chunk = bytes;
        self.chunk_start = start;
        if let Some(progress) = &self.progress {
            progress(start, end, Some(self.total_size));
        }
    }

    /// Drop the prefetch slot only once its worker has finished; a still-running
    /// worker stays tracked so at most one range download is ever in flight,
    /// no matter how often the caller seeks around it.
    fn discard_prefetch_if_done(&mut self) {
        if let Some(prefetch) = &self.prefetch
            && prefetch.slot.try_take().is_some()
        {
            self.prefetch = None;
        }
    }

    fn fetch_chunk(&mut self, start: u64) -> IoResult<()> {
        if let Some(prefetch) = self.prefetch.take_if(|p| p.start == start) {
            match prefetch.slot.take_blocking() {
                Ok(bytes) => {
                    self.install_chunk(start, bytes);
                    return Ok(());
                }
                Err(error) => {
                    tracing::debug!(%error, start, "prefetched range failed; refetching inline");
                }
            }
        }
        self.discard_prefetch_if_done();
        let bytes = fetch_range(&self.client, &self.url, start, self.total_size)?;
        self.install_chunk(start, bytes);
        Ok(())
    }

    /// Kick off the next window's download once sequential reading is past the
    /// middle of the current one. At most one worker is in flight: a slot left
    /// over from before a seek keeps blocking new spawns until its download
    /// completes, and is discarded the first time it's seen finished.
    fn maybe_prefetch_next(&mut self) {
        let next_start = self.chunk_start + self.chunk.len() as u64;
        if next_start >= self.total_size
            || self.pos < self.chunk_start + (self.chunk.len() / 2) as u64
        {
            return;
        }
        if self.prefetch.is_some() {
            if self
                .prefetch
                .as_ref()
                .is_some_and(|p| p.start == next_start)
            {
                return;
            }
            self.discard_prefetch_if_done();
            if self.prefetch.is_some() {
                return;
            }
        }
        let slot = std::sync::Arc::new(PrefetchSlot::default());
        let worker_slot = slot.clone();
        let client = self.client.clone();
        let url = self.url.clone();
        let total_size = self.total_size;
        std::thread::spawn(move || {
            worker_slot.put(fetch_range(&client, &url, next_start, total_size));
        });
        self.prefetch = Some(Prefetch {
            start: next_start,
            slot,
        });
    }

    fn pos_in_cache(&self, pos: u64) -> bool {
        !self.chunk.is_empty()
            && pos >= self.chunk_start
            && pos < self.chunk_start + self.chunk.len() as u64
    }
}

/// One pooled client per user-agent string, shared by every source for the
/// life of the process. A fresh client per track meant a fresh connection pool
/// per track: every song opened with a full TCP + TLS handshake (on Android
/// that includes the platform-verifier JNI round-trip), which is most of the
/// "takes forever to start" on high-RTT phone links. Keep-alive across tracks
/// makes each chunk a single pipelined request on a warm connection.
fn shared_client(ua: &str) -> IoResult<reqwest::blocking::Client> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CLIENTS: OnceLock<Mutex<HashMap<String, reqwest::blocking::Client>>> = OnceLock::new();
    let clients = CLIENTS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut clients = clients.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(client) = clients.get(ua) {
        return Ok(client.clone());
    }
    let client = reqwest::blocking::Client::builder()
        .tcp_nodelay(true)
        .user_agent(ua)
        .timeout(REQUEST_TIMEOUT)
        .pool_idle_timeout(Duration::from_secs(300))
        .build()
        .map_err(IoError::other)?;
    clients.insert(ua.to_string(), client.clone());
    Ok(client)
}

fn fetch_range(
    client: &reqwest::blocking::Client,
    url: &str,
    start: u64,
    total_size: u64,
) -> IoResult<Vec<u8>> {
    let end = (start + CHUNK as u64 - 1).min(total_size - 1);
    let resp = client
        .get(url)
        .header("Range", format!("bytes={start}-{end}"))
        .send()
        .map_err(|e| IoError::other(e.without_url()))?;
    if resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(IoError::other(format!(
            "range fetch {start}-{end} expected HTTP 206, got {}",
            resp.status()
        )));
    }
    let bytes = resp.bytes().map_err(|e| IoError::other(e.without_url()))?;
    let expected = (end - start + 1) as usize;
    if bytes.len() != expected {
        return Err(IoError::new(
            ErrorKind::UnexpectedEof,
            format!(
                "range fetch {start}-{end} returned {} bytes, expected {expected}",
                bytes.len()
            ),
        ));
    }
    Ok(bytes.to_vec())
}

impl Read for RangeStreamSource {
    fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
        if self.pos >= self.total_size {
            return Ok(0);
        }
        if !self.pos_in_cache(self.pos) {
            self.fetch_chunk(self.pos)?;
            if self.chunk.is_empty() {
                return Ok(0);
            }
        }
        let offset = (self.pos - self.chunk_start) as usize;
        let available = self.chunk.len() - offset;
        let to_copy = available.min(buf.len());
        buf[..to_copy].copy_from_slice(&self.chunk[offset..offset + to_copy]);
        self.pos += to_copy as u64;
        self.maybe_prefetch_next();
        Ok(to_copy)
    }
}

impl Seek for RangeStreamSource {
    fn seek(&mut self, p: SeekFrom) -> IoResult<u64> {
        let new_pos: i64 = match p {
            SeekFrom::Start(n) => n as i64,
            SeekFrom::Current(n) => self.pos as i64 + n,
            SeekFrom::End(n) => self.total_size as i64 + n,
        };
        if new_pos < 0 {
            return Err(IoError::new(
                ErrorKind::InvalidInput,
                "seek to negative position",
            ));
        }
        self.pos = new_pos as u64;
        Ok(self.pos)
    }
}

fn parse_total_size(resp: &reqwest::blocking::Response) -> Option<u64> {
    // Content-Range: "bytes 0-0/12345" — the part after '/' is the total.
    // Content-Length on this 206 response is only the one-byte probe length.
    resp.headers()
        .get("content-range")
        .and_then(|value| value.to_str().ok())
        .and_then(total_size_from_content_range)
}

fn total_size_from_content_range(value: &str) -> Option<u64> {
    let (_, total) = value.rsplit_once('/')?;
    if total == "*" {
        return None;
    }
    total.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::total_size_from_content_range;

    #[test]
    fn parses_total_from_content_range() {
        assert_eq!(
            total_size_from_content_range("bytes 0-0/123456"),
            Some(123_456)
        );
    }

    #[test]
    fn rejects_unknown_or_malformed_content_range_totals() {
        assert_eq!(total_size_from_content_range("bytes 0-0/*"), None);
        assert_eq!(total_size_from_content_range("bytes 0-0/not-a-size"), None);
        assert_eq!(total_size_from_content_range("123456"), None);
    }
}
