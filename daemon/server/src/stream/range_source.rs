//! A seekable byte source over HTTP Range requests that downloads the whole file, chunk after chunk, from the moment it opens.

use std::io::{Error as IoError, ErrorKind, Read, Result as IoResult, Seek, SeekFrom};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use super::stream_buffer::{BufferProgressCallback, Captured, CompleteSink};

const CHUNK: usize = 512 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// A CDN closes an idle keep-alive well inside a minute, and a pooled socket that outlives the peer's burns a whole timeout.
const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(20);

/// The whole file is held in memory, so a larger one streams progressively instead.
const MAX_WHOLE_FILE: u64 = 1024 * 1024 * 1024;

/// How long the fill waits after a chunk it fetched on its own failed.
const RETRY_AFTER: Duration = Duration::from_secs(2);

/// Failed chunks in a row after which the fill waits for a reader to ask for one.
const MAX_BACKGROUND_FAILURES: u32 = 4;

/// How long a reader sleeps between looks at the chunk it waits for.
const READ_WAIT: Duration = Duration::from_millis(250);

#[derive(Default)]
struct Fill {
    data: Vec<u8>,
    have: Vec<bool>,
    missing: usize,
    /// The chunk a reader is blocked on, fetched before any other.
    demand: Option<usize>,
    in_flight: Option<usize>,
    /// Where the fill goes on from after the chunk it fetched last.
    cursor: usize,
    /// A chunk a reader asked for whose fetch failed, for that reader to hear.
    failed: Option<(usize, ErrorKind, String)>,
    background_failures: u32,
    retry_at: Option<Instant>,
    closed: bool,
}

impl Fill {
    /// The first missing chunk from the cursor on, else the first one before it: a gap a seek skipped fills after the end.
    fn next_missing(&self) -> Option<usize> {
        let after = self.cursor.min(self.have.len());
        (after..self.have.len())
            .chain(0..after)
            .find(|&chunk| !self.have[chunk])
    }
}

struct Shared {
    fill: Mutex<Fill>,
    changed: Condvar,
}

enum Step {
    Fetch { chunk: usize, demanded: bool },
    Complete,
    Closed,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Fill> {
        self.fill.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn next_step(&self) -> Step {
        let mut fill = self.lock();
        loop {
            if fill.closed {
                return Step::Closed;
            }
            if fill.missing == 0 {
                return Step::Complete;
            }
            if let Some(chunk) = fill.demand.take() {
                if !fill.have[chunk] {
                    fill.in_flight = Some(chunk);
                    return Step::Fetch {
                        chunk,
                        demanded: true,
                    };
                }
                continue;
            }
            if fill.background_failures >= MAX_BACKGROUND_FAILURES {
                fill = self
                    .changed
                    .wait(fill)
                    .unwrap_or_else(PoisonError::into_inner);
                continue;
            }
            if let Some(wait) = fill
                .retry_at
                .and_then(|at| at.checked_duration_since(Instant::now()))
                .filter(|wait| !wait.is_zero())
            {
                fill = self
                    .changed
                    .wait_timeout(fill, wait)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
                continue;
            }
            let Some(chunk) = fill.next_missing() else {
                return Step::Complete;
            };
            fill.in_flight = Some(chunk);
            return Step::Fetch {
                chunk,
                demanded: false,
            };
        }
    }

    /// Store what one fetch brought, and answer the byte range it filled.
    fn land(&self, chunk: usize, demanded: bool, result: IoResult<Vec<u8>>) -> Option<(u64, u64)> {
        let mut fill = self.lock();
        fill.in_flight = None;
        let landed = match result {
            Ok(bytes) => {
                let start = chunk * CHUNK;
                let end = start + bytes.len();
                fill.data[start..end].copy_from_slice(&bytes);
                if !fill.have[chunk] {
                    fill.have[chunk] = true;
                    fill.missing -= 1;
                }
                fill.cursor = chunk + 1;
                fill.background_failures = 0;
                fill.retry_at = None;
                if fill.failed.as_ref().is_some_and(|failed| failed.0 == chunk) {
                    fill.failed = None;
                }
                Some((start as u64, end as u64))
            }
            Err(error) => {
                tracing::debug!(%error, chunk, demanded, "a range chunk failed");
                if demanded {
                    fill.failed = Some((chunk, error.kind(), error.to_string()));
                } else {
                    fill.background_failures += 1;
                }
                fill.retry_at = Some(Instant::now() + RETRY_AFTER);
                None
            }
        };
        self.changed.notify_all();
        landed
    }
}

/// What the fill thread needs, held until the first read or [`RangeStreamSource::capture`] starts it.
struct FillJob {
    client: reqwest::blocking::Client,
    url: String,
    content_type: Option<String>,
    progress: Option<BufferProgressCallback>,
    complete: Option<CompleteSink>,
}

pub struct RangeStreamSource {
    shared: Arc<Shared>,
    total_size: u64,
    pos: u64,
    url: String,
    client: reqwest::blocking::Client,
    /// Behind a lock only so the source stays `Sync`, as a decoder needs; a sink need not be.
    pending: Mutex<Option<FillJob>>,
}

impl RangeStreamSource {
    /// Probe the URL with a one-byte range to learn its size and confirm range support; nothing more is fetched until it is read.
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

        // The first request after the gap between tracks, so the one a dead pooled connection strands.
        let resp = send_range(&client, &url, "bytes=0-0")?;
        let status = resp.status();
        if status != reqwest::StatusCode::PARTIAL_CONTENT {
            return Err(IoError::new(
                ErrorKind::Unsupported,
                format!("server ignored range probe (HTTP {status})"),
            ));
        }
        let total_size = parse_total_size(&resp)
            .ok_or_else(|| IoError::other("range response didn't expose total size"))?;
        if total_size > MAX_WHOLE_FILE {
            return Err(IoError::new(
                ErrorKind::Unsupported,
                format!("{total_size} bytes is too large to hold whole"),
            ));
        }
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);

        let chunks = (total_size as usize).div_ceil(CHUNK);
        let shared = Arc::new(Shared {
            fill: Mutex::new(Fill {
                data: vec![0; total_size as usize],
                have: vec![false; chunks],
                missing: chunks,
                ..Fill::default()
            }),
            changed: Condvar::new(),
        });
        Ok(Self {
            shared,
            total_size,
            pos: 0,
            pending: Mutex::new(Some(FillJob {
                client: client.clone(),
                url: url.clone(),
                content_type,
                progress,
                complete: None,
            })),
            url,
            client,
        })
    }

    pub fn total_size(&self) -> u64 {
        self.total_size
    }

    /// Ask for the last byte. googlevideo can serve the head of a stream and
    /// answer 403 past it, so passing the head probe says nothing about deep
    /// reads, and the first one is symphonia's own read of the webm tail.
    pub fn check_tail(&self) -> IoResult<()> {
        let last = self.total_size.saturating_sub(1);
        let resp = send_range(&self.client, &self.url, &format!("bytes={last}-{last}"))?;
        if resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
            return Err(IoError::new(
                ErrorKind::PermissionDenied,
                format!("tail range refused (HTTP {})", resp.status()),
            ));
        }
        Ok(())
    }

    /// Hand the whole file to `sink` once every chunk has arrived, and start fetching them; a source dropped first never calls it.
    pub fn capture(&mut self, sink: CompleteSink) {
        if let Some(job) = self
            .pending
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .as_mut()
        {
            job.complete = Some(sink);
        }
        self.start();
    }

    fn start(&mut self) {
        let Some(job) = self
            .pending
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        else {
            return;
        };
        if self.total_size == 0 {
            return;
        }
        let shared = self.shared.clone();
        let total_size = self.total_size;
        let spawned = std::thread::Builder::new()
            .name("range-fill".into())
            .spawn(move || run_fill(shared, job, total_size));
        if let Err(error) = spawned {
            tracing::warn!(%error, "the range fill thread failed to start");
        }
    }
}

fn run_fill(shared: Arc<Shared>, job: FillJob, total_size: u64) {
    let FillJob {
        client,
        url,
        content_type,
        progress,
        complete,
    } = job;
    loop {
        match shared.next_step() {
            Step::Closed => return,
            Step::Complete => break,
            Step::Fetch { chunk, demanded } => {
                let start = chunk as u64 * CHUNK as u64;
                let result = fetch_range(&client, &url, start, total_size);
                if let Some((start, end)) = shared.land(chunk, demanded, result)
                    && let Some(progress) = &progress
                {
                    progress(start, end, Some(total_size));
                }
            }
        }
    }
    let Some(complete) = complete else {
        return;
    };
    let bytes = shared.lock().data.clone();
    complete(Captured {
        bytes,
        content_type,
    });
}

impl Drop for RangeStreamSource {
    fn drop(&mut self) {
        self.shared.lock().closed = true;
        self.shared.changed.notify_all();
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
    use std::sync::OnceLock;
    static CLIENTS: OnceLock<Mutex<HashMap<String, reqwest::blocking::Client>>> = OnceLock::new();
    let clients = CLIENTS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut clients = clients.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(client) = clients.get(ua) {
        return Ok(client.clone());
    }
    let client = utils::http::blocking_builder()
        .tcp_nodelay(true)
        .user_agent(ua)
        .timeout(REQUEST_TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .pool_idle_timeout(POOL_IDLE_TIMEOUT)
        .build()
        .map_err(IoError::other)?;
    clients.insert(ua.to_string(), client.clone());
    Ok(client)
}

/// Send a range request, once more if the connection failed before answering.
///
/// A pooled connection the peer has already closed fails on the send rather
/// than with a status, and the retry opens a fresh one. Exactly one retry: a
/// second failure is the network, not a stale socket, and the caller needs to
/// hear about it rather than wait through another timeout.
fn send_range(
    client: &reqwest::blocking::Client,
    url: &str,
    range: &str,
) -> IoResult<reqwest::blocking::Response> {
    let send = || client.get(url).header("Range", range).send();
    // `without_url`: a stream URL can carry credentials in its userinfo, and a
    // reqwest error prints the URL it failed on.
    match send() {
        Ok(response) => return Ok(response),
        Err(error) if error.is_request() || error.is_timeout() => {
            tracing::debug!(range, "no response on a pooled connection; retrying once")
        }
        Err(error) => return Err(IoError::other(error.without_url())),
    }
    send().map_err(|error| {
        let error = error.without_url();
        tracing::warn!(range, %error, "the retried range request failed too");
        let kind = if error.is_connect() {
            ErrorKind::NotConnected
        } else if error.is_timeout() {
            ErrorKind::TimedOut
        } else {
            ErrorKind::Other
        };
        IoError::new(kind, error)
    })
}

fn fetch_range(
    client: &reqwest::blocking::Client,
    url: &str,
    start: u64,
    total_size: u64,
) -> IoResult<Vec<u8>> {
    let end = (start + CHUNK as u64 - 1).min(total_size - 1);
    let resp = send_range(client, url, &format!("bytes={start}-{end}"))?;
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
        if self.pos >= self.total_size || buf.is_empty() {
            return Ok(0);
        }
        self.start();
        let chunk = (self.pos / CHUNK as u64) as usize;
        let mut fill = self.shared.lock();
        loop {
            if fill.have[chunk] {
                let chunk_end = ((chunk + 1) * CHUNK).min(self.total_size as usize);
                let at = self.pos as usize;
                let n = buf.len().min(chunk_end - at);
                buf[..n].copy_from_slice(&fill.data[at..at + n]);
                self.pos += n as u64;
                return Ok(n);
            }
            if let Some((failed, kind, message)) = fill.failed.take()
                && failed == chunk
            {
                return Err(IoError::new(kind, message));
            }
            if fill.in_flight != Some(chunk) && fill.demand != Some(chunk) {
                fill.demand = Some(chunk);
                self.shared.changed.notify_all();
            }
            fill = self
                .shared
                .changed
                .wait_timeout(fill, READ_WAIT)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
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
    use std::io::{BufRead, BufReader, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::mpsc;

    use super::*;

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

    /// A file of `len` bytes whose every byte says where it is.
    fn body(len: usize) -> Vec<u8> {
        (0..len).map(|at| (at * 7 % 251) as u8).collect()
    }

    /// Serves `body` by range on loopback, telling `seen` each range it was asked for; with `gate`, each chunk waits to be let through.
    struct Server {
        url: String,
        seen: mpsc::Receiver<(u64, u64)>,
        gate: Option<mpsc::SyncSender<()>>,
    }

    fn serve(body: Vec<u8>, gated: bool) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let url = format!("http://{}/track", listener.local_addr().expect("addr"));
        let (seen_tx, seen) = mpsc::channel();
        let (gate, pass) = match gated {
            true => {
                let (tx, rx) = mpsc::sync_channel::<()>(64);
                (Some(tx), Some(rx))
            }
            false => (None, None),
        };
        let body = Arc::new(body);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let Some((start, end)) = read_range(&stream) else {
                    continue;
                };
                if seen_tx.send((start, end)).is_err() {
                    return;
                }
                // The size probe and the tail check are answered at once; a chunk waits its turn.
                if start != end
                    && let Some(pass) = &pass
                    && pass.recv().is_err()
                {
                    return;
                }
                answer(stream, &body, start, end);
            }
        });
        Server { url, seen, gate }
    }

    fn read_range(stream: &TcpStream) -> Option<(u64, u64)> {
        let mut reader = BufReader::new(stream);
        let mut range = None;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).ok()? == 0 {
                return None;
            }
            let line = line.trim_end();
            if line.is_empty() {
                return range;
            }
            if let Some(value) = line
                .to_ascii_lowercase()
                .strip_prefix("range: bytes=")
                .map(str::to_string)
            {
                let (start, end) = value.split_once('-')?;
                range = Some((start.parse().ok()?, end.parse().ok()?));
            }
        }
    }

    fn answer(mut stream: TcpStream, body: &[u8], start: u64, end: u64) {
        let slice = &body[start as usize..=end as usize];
        let head = format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Type: audio/webm\r\nContent-Length: {}\r\n\
             Content-Range: bytes {start}-{end}/{}\r\nConnection: close\r\n\r\n",
            slice.len(),
            body.len()
        );
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(slice);
    }

    fn chunk_range(chunk: u64, len: usize) -> (u64, u64) {
        let start = chunk * CHUNK as u64;
        (start, (start + CHUNK as u64 - 1).min(len as u64 - 1))
    }

    fn capture_into(source: &mut RangeStreamSource) -> mpsc::Receiver<Captured> {
        let (tx, rx) = mpsc::channel();
        source.capture(Box::new(move |captured| {
            let _ = tx.send(captured);
        }));
        rx
    }

    const WAIT: Duration = Duration::from_secs(10);

    #[test]
    fn every_chunk_is_fetched_back_to_back_without_a_read() {
        let len = 3 * CHUNK + 1000;
        let server = serve(body(len), false);
        let mut source = RangeStreamSource::new(server.url.clone(), None).expect("open");

        let captured = capture_into(&mut source).recv_timeout(WAIT).expect("whole");

        assert_eq!(server.seen.recv_timeout(WAIT), Ok((0, 0)), "the size probe");
        let fetched: Vec<(u64, u64)> = server.seen.try_iter().collect();
        assert_eq!(
            fetched,
            (0..4)
                .map(|chunk| chunk_range(chunk, len))
                .collect::<Vec<_>>(),
            "no chunk waited on the playhead"
        );
        assert_eq!(source.pos, 0);
        assert_eq!(captured.bytes, body(len), "byte for byte");
        assert_eq!(captured.content_type.as_deref(), Some("audio/webm"));
    }

    #[test]
    fn a_seek_past_the_fill_is_fetched_next_and_the_gap_after_the_end() {
        let len = 8 * CHUNK;
        let server = serve(body(len), true);
        let gate = server.gate.clone().expect("gated");
        let mut source = RangeStreamSource::new(server.url.clone(), None).expect("open");
        let captured = capture_into(&mut source);
        assert_eq!(server.seen.recv_timeout(WAIT), Ok((0, 0)));

        gate.send(()).expect("let chunk 0 through");
        assert_eq!(server.seen.recv_timeout(WAIT), Ok(chunk_range(0, len)));
        assert_eq!(server.seen.recv_timeout(WAIT), Ok(chunk_range(1, len)));
        // Chunk 1 is in flight while the listener jumps to chunk 6.
        source
            .seek(SeekFrom::Start(6 * CHUNK as u64 + 10))
            .expect("seek");
        let reader = std::thread::spawn(move || {
            let mut buf = [0u8; 16];
            let n = source.read(&mut buf).expect("read past the seek");
            (source, buf[..n].to_vec())
        });
        std::thread::sleep(Duration::from_millis(200));
        for _ in 0..7 {
            gate.send(()).expect("let a chunk through");
        }

        let order: Vec<u64> = (0..6)
            .map(|_| server.seen.recv_timeout(WAIT).expect("a chunk").0 / CHUNK as u64)
            .collect();
        assert_eq!(order, [6, 7, 2, 3, 4, 5], "the seek first, then the gap");
        let (_source, read) = reader.join().expect("reader");
        let at = 6 * CHUNK + 10;
        assert_eq!(read, body(len)[at..at + read.len()]);
        assert_eq!(captured.recv_timeout(WAIT).expect("whole").bytes, body(len));
    }

    #[test]
    fn a_source_dropped_before_the_last_chunk_hands_nothing_over() {
        let len = 4 * CHUNK;
        let server = serve(body(len), true);
        let gate = server.gate.clone().expect("gated");
        let mut source = RangeStreamSource::new(server.url.clone(), None).expect("open");
        let captured = capture_into(&mut source);
        gate.send(()).expect("chunk 0");
        assert_eq!(server.seen.recv_timeout(WAIT), Ok((0, 0)));
        assert_eq!(server.seen.recv_timeout(WAIT), Ok(chunk_range(0, len)));

        drop(source);
        for _ in 0..4 {
            let _ = gate.send(());
        }

        assert!(
            captured.recv_timeout(Duration::from_secs(2)).is_err(),
            "a skipped track is not kept"
        );
    }
}
