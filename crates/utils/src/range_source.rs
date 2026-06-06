//! HTTP Range-backed seekable byte source.
//!
//! Used for YouTube Music (and any other URL where the server returns
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
//! initial probe fetch. If the server doesn't include it, we fall back to
//! a HEAD request. If both fail, this source can't be constructed.

use std::io::{Error as IoError, ErrorKind, Read, Result as IoResult, Seek, SeekFrom};
use std::time::Duration;

const CHUNK: usize = 512 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

pub struct RangeStreamSource {
    url: String,
    client: reqwest::blocking::Client,
    total_size: u64,
    pos: u64,
    chunk: Vec<u8>,
    chunk_start: u64,
}

impl RangeStreamSource {
    /// Probe the URL with a `Range: bytes=0-0` HEAD-equivalent to learn its
    /// total size and confirm Range support. Returns the source positioned
    /// at byte 0 with an empty cache.
    pub fn new(url: String, user_agent: Option<String>) -> IoResult<Self> {
        let ua = user_agent
            .unwrap_or_else(|| concat!("Kopuz/", env!("CARGO_PKG_VERSION")).to_string());
        let client = reqwest::blocking::Client::builder()
            .tcp_nodelay(true)
            .user_agent(ua)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(IoError::other)?;

        // One-byte probe — cheap, and the server returns the full
        // `Content-Range: bytes 0-0/<TOTAL>` we want.
        let resp = client
            .get(&url)
            .header("Range", "bytes=0-0")
            .send()
            .map_err(IoError::other)?;
        let status = resp.status();
        if !status.is_success() {
            return Err(IoError::other(format!(
                "range probe HTTP {status}"
            )));
        }
        let total_size = parse_total_size(&resp).ok_or_else(|| {
            IoError::other("server didn't expose total size on range probe")
        })?;

        Ok(Self {
            url,
            client,
            total_size,
            pos: 0,
            chunk: Vec::with_capacity(CHUNK),
            chunk_start: 0,
        })
    }

    pub fn total_size(&self) -> u64 {
        self.total_size
    }

    fn fetch_chunk(&mut self, start: u64) -> IoResult<()> {
        let end = (start + CHUNK as u64 - 1).min(self.total_size - 1);
        let resp = self
            .client
            .get(&self.url)
            .header("Range", format!("bytes={start}-{end}"))
            .send()
            .map_err(IoError::other)?;
        if !resp.status().is_success() {
            return Err(IoError::other(format!(
                "range fetch {start}-{end} HTTP {}",
                resp.status()
            )));
        }
        let bytes = resp.bytes().map_err(IoError::other)?;
        self.chunk.clear();
        self.chunk.extend_from_slice(&bytes);
        self.chunk_start = start;
        Ok(())
    }

    fn pos_in_cache(&self, pos: u64) -> bool {
        !self.chunk.is_empty()
            && pos >= self.chunk_start
            && pos < self.chunk_start + self.chunk.len() as u64
    }
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
    // Prefer Content-Range: "bytes 0-0/12345" — the part after '/' is the
    // total. Fall back to Content-Length only if Range wasn't honoured (in
    // which case the server gave us the whole body, and Content-Length is
    // the full file size).
    if let Some(v) = resp.headers().get("content-range") {
        if let Ok(s) = v.to_str() {
            if let Some(slash) = s.rfind('/') {
                let tail = &s[slash + 1..];
                if tail != "*" {
                    if let Ok(n) = tail.parse() {
                        return Some(n);
                    }
                }
            }
        }
    }
    resp.content_length()
}
