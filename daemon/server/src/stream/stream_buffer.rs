use std::cmp::min;
use std::io::{Error as IoError, ErrorKind, Read, Result as IoResult, Seek, SeekFrom};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, Notify};
use tracing::Instrument;

const MIN_PREBUFFER_BYTES: usize = 256 * 1024;

const MIN_BUFFER_AHEAD: usize = 128 * 1024;

const MAX_BUFFER_SIZE: usize = 1024 * 1024 * 1024;
const PROGRESS_REPORT_STEP: usize = 64 * 1024;

/// Reports one downloaded byte range as `[start, end)` plus the total length
/// when the server exposes it.
pub type BufferProgressCallback = Arc<dyn Fn(u64, u64, Option<u64>) + Send + Sync>;

/// A track's whole file, as its stream downloaded it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Captured {
    pub bytes: Vec<u8>,
    pub content_type: Option<String>,
}

/// Takes a stream's whole file once every byte has arrived; a stream dropped or cut short first drops it uncalled.
pub type CompleteSink = Box<dyn FnOnce(Captured) + Send>;

/// Shared state between the async downloader and sync readers.
///
/// Fields:
/// - `buffer`: accumulated audio bytes
/// - `done`: HTTP stream finished (success or error)
/// - `error`: reason for failure, if any
/// - `total_size`: Content-Length header value (may be unknown for radio/streams)
/// - `response_ready`: response headers arrived, even when no Content-Length exists
/// - `prebuffer_ready`: enough data buffered to start playback safely
struct SharedState {
    buffer: Vec<u8>,
    done: bool,
    error: Option<(ErrorKind, String)>,
    total_size: Option<u64>,
    response_ready: bool,
    prebuffer_ready: bool,
}

pub struct StreamBuffer {
    state: Arc<(Mutex<SharedState>, Notify)>,
    pos: u64,
}

fn configured(
    builder: reqwest::ClientBuilder,
    user_agent: Option<String>,
) -> reqwest::Result<reqwest::Client> {
    let ua = user_agent.unwrap_or_else(|| concat!("Kopuz/", env!("CARGO_PKG_VERSION")).to_string());
    builder.tcp_nodelay(true).user_agent(ua).build()
}

/// The client a progressive stream of a track is fetched with.
pub fn stream_client(user_agent: Option<String>) -> reqwest::Client {
    configured(utils::http::builder(), user_agent).unwrap_or_else(|_| utils::http::client())
}

/// The client a radio stream is fetched with: native TLS on desktop, since some broadcaster CDNs reset rustls's handshake.
pub fn radio_client(user_agent: Option<String>) -> reqwest::Client {
    #[cfg(not(target_os = "android"))]
    match configured(utils::http::native_tls_builder(), user_agent.clone()) {
        Ok(client) => return client,
        Err(error) => tracing::warn!(%error, "native TLS unavailable; streaming radio over rustls"),
    }
    stream_client(user_agent)
}

impl StreamBuffer {
    /// `runtime` is the handle the background download runs on. It's passed in
    /// (not `Handle::current()`) because construction happens on the engine's
    /// decode worker — a plain thread with no runtime in scope. The sync reader
    /// side uses `blocking_lock()` and needs no runtime.
    ///
    /// `icy_tx`: when set, ICY metadata is requested, stripped from the
    /// audio, and published on the channel.
    pub fn new(
        url: String,
        is_radio: bool,
        client: reqwest::Client,
        icy_tx: Option<tokio::sync::watch::Sender<super::icy::IcyMeta>>,
        runtime: tokio::runtime::Handle,
    ) -> Self {
        Self::with_progress(url, is_radio, client, icy_tx, runtime, None)
    }

    pub fn with_progress(
        url: String,
        is_radio: bool,
        client: reqwest::Client,
        icy_tx: Option<tokio::sync::watch::Sender<super::icy::IcyMeta>>,
        runtime: tokio::runtime::Handle,
        progress: Option<BufferProgressCallback>,
    ) -> Self {
        Self::open(url, is_radio, client, icy_tx, runtime, progress, None)
    }

    /// A track's progressive stream that hands its whole file to `complete` once the body has arrived in full.
    pub fn capturing(
        url: String,
        client: reqwest::Client,
        runtime: tokio::runtime::Handle,
        progress: Option<BufferProgressCallback>,
        complete: Option<CompleteSink>,
    ) -> Self {
        Self::open(url, false, client, None, runtime, progress, complete)
    }

    fn open(
        url: String,
        is_radio: bool,
        client: reqwest::Client,
        icy_tx: Option<tokio::sync::watch::Sender<super::icy::IcyMeta>>,
        runtime: tokio::runtime::Handle,
        progress: Option<BufferProgressCallback>,
        complete: Option<CompleteSink>,
    ) -> Self {
        let prebuffer_size = if is_radio {
            16 * 1024
        } else {
            MIN_PREBUFFER_BYTES
        };

        let state = Arc::new((
            Mutex::new(SharedState {
                buffer: Vec::with_capacity(prebuffer_size * 4),
                done: false,
                error: None,
                total_size: None,
                response_ready: false,
                prebuffer_ready: false,
            }),
            Notify::new(),
        ));

        let state_clone = state.clone();

        runtime.spawn(
            async move {
                // Radio directories often hand out a playlist (.pls/.m3u par example)
                // instead of the stream; follow it to its first entry.
                let mut url = url;
                let mut hops = 0u8;
                let result = loop {
                    if hops > 3 {
                        break Err((ErrorKind::Other, "too many playlist redirects".to_string()));
                    }
                    hops += 1;
                    let mut request = client.get(&url);
                    if icy_tx.is_some() {
                        request = request.header("Icy-MetaData", "1");
                    }
                    match request.send().await {
                        Ok(response) if response.status().is_success() => {
                            let content_type = response
                                .headers()
                                .get("content-type")
                                .and_then(|v| v.to_str().ok());
                            if !utils::playlist::is_playlist(content_type, response.url().path()) {
                                break Ok(response);
                            }
                            match response.text().await.ok().as_deref().and_then(|text| {
                                utils::playlist::first_stream_url(text)
                                    .filter(|next| !utils::playlist::is_playlist(None, next))
                            }) {
                                Some(next) => {
                                    tracing::debug!(
                                        from = %utils::redact::redact_url(&url),
                                        to = %utils::redact::redact_url(&next),
                                        "resolved playlist to stream URL",
                                    );
                                    url = next;
                                }
                                None => {
                                    break Err((
                                        ErrorKind::Other,
                                        "playlist has no playable stream URL".to_string(),
                                    ));
                                }
                            }
                        }
                        Ok(response) => {
                            break Err((
                                status_kind(response.status()),
                                format!("HTTP {}", response.status()),
                            ));
                        }
                        // `without_url`: the URL a request failed on can carry
                        // credentials, and this string reaches the player UI.
                        Err(e) => {
                            let e = e.without_url();
                            // reqwest's own text stops at "error sending request"; the cause is in its sources.
                            let mut chain = e.to_string();
                            let mut source = std::error::Error::source(&e);
                            while let Some(cause) = source {
                                chain = format!("{chain}: {cause}");
                                source = cause.source();
                            }
                            break Err((request_kind(&e), chain));
                        }
                    }
                };

                match result {
                    Ok(mut response) => {
                        tracing::trace!(
                            status = %response.status(),
                            content_length = ?response.content_length(),
                            content_type = ?response
                                .headers()
                                .get("content-type")
                                .and_then(|v| v.to_str().ok()),
                            "stream buffer HTTP response",
                        );
                        let total_size = response.content_length();
                        let content_type = response
                            .headers()
                            .get(reqwest::header::CONTENT_TYPE)
                            .and_then(|value| value.to_str().ok())
                            .map(str::to_string);
                        {
                            let (lock, notify) = &*state_clone;
                            let mut state = lock.lock().await;
                            state.total_size = total_size;
                            state.response_ready = true;
                            notify.notify_waiters();
                        }

                        let mut icy = icy_tx.and_then(|tx| {
                            let metaint = response
                                .headers()
                                .get("icy-metaint")
                                .and_then(|v| v.to_str().ok())
                                .and_then(|v| v.trim().parse::<usize>().ok())
                                .filter(|m| *m > 0)?;
                            tracing::debug!(metaint, "ICY metadata enabled for radio stream");
                            Some((super::icy::IcyDeinterleaver::new(metaint), tx))
                        });

                        let mut total_buffered = 0usize;
                        let mut last_progress_report = 0usize;
                        let mut audio_scratch = Vec::new();
                        let mut whole = false;

                        loop {
                            let chunk = match response.chunk().await {
                                Ok(Some(chunk)) => chunk,
                                Ok(None) => {
                                    whole = true;
                                    break;
                                }
                                Err(_) => break,
                            };
                            if Arc::strong_count(&state_clone) == 1 {
                                break;
                            }

                            let data: &[u8] = match icy.as_mut() {
                                Some((parser, tx)) => {
                                    audio_scratch.clear();
                                    audio_scratch.reserve(chunk.len());
                                    if let Some(meta) = parser.push(&chunk, &mut audio_scratch) {
                                        tx.send_replace(meta);
                                    }
                                    &audio_scratch
                                }
                                None => &chunk,
                            };
                            let chunk_len = data.len();

                            if total_buffered + chunk_len > MAX_BUFFER_SIZE {
                                let (lock, notify) = &*state_clone;
                                let mut state = lock.lock().await;
                                state.error = Some((
                                    ErrorKind::Other,
                                    "Buffer limit exceeded (1GB)".to_string(),
                                ));
                                state.done = true;
                                state.prebuffer_ready = true;
                                notify.notify_waiters();
                                break;
                            }

                            {
                                let (lock, notify) = &*state_clone;
                                let mut state = lock.lock().await;
                                state.buffer.extend_from_slice(data);
                                total_buffered += chunk_len;

                                if !state.prebuffer_ready {
                                    let is_small_file = state
                                        .total_size
                                        .map(|s| s <= prebuffer_size as u64)
                                        .unwrap_or(false);

                                    if total_buffered >= prebuffer_size || is_small_file {
                                        state.prebuffer_ready = true;
                                    }
                                }

                                notify.notify_waiters();
                            }

                            if total_buffered.saturating_sub(last_progress_report)
                                >= PROGRESS_REPORT_STEP
                            {
                                if let Some(progress) = &progress {
                                    progress(0, total_buffered as u64, total_size);
                                }
                                last_progress_report = total_buffered;
                            }
                        }

                        let (lock, notify) = &*state_clone;
                        let mut state = lock.lock().await;
                        state.done = true;
                        state.prebuffer_ready = true;
                        notify.notify_waiters();
                        let whole = whole
                            && total_buffered > 0
                            && total_size.is_none_or(|size| size == total_buffered as u64);
                        let captured = complete
                            .filter(|_| whole)
                            .map(|complete| (complete, state.buffer.clone()));
                        drop(state);
                        if let Some((complete, bytes)) = captured {
                            tokio::task::spawn_blocking(move || {
                                complete(Captured {
                                    bytes,
                                    content_type,
                                })
                            });
                        }
                        if total_buffered > 0
                            && let Some(progress) = &progress
                        {
                            progress(
                                0,
                                total_buffered as u64,
                                total_size.or(Some(total_buffered as u64)),
                            );
                        }
                    }
                    Err(e) => {
                        let (lock, notify) = &*state_clone;
                        let mut state = lock.lock().await;
                        state.error = Some(e);
                        state.done = true;
                        state.prebuffer_ready = true;
                        notify.notify_waiters();
                    }
                }
            }
            .instrument(tracing::info_span!("player.stream_buffer")),
        );

        Self { state, pos: 0 }
    }

    fn wait_for_prebuffer(&self) {
        let (lock, _notify) = &*self.state;
        loop {
            let state = lock.blocking_lock();
            if state.prebuffer_ready || state.done {
                return;
            }
            drop(state);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Wait until the HTTP response headers tell us whether a total size is
    /// available. A chunked response has no Content-Length, so it must not wait
    /// for the body to finish downloading.
    pub fn wait_for_response_headers(&self) {
        let (lock, _notify) = &*self.state;
        loop {
            let state = lock.blocking_lock();
            if state.response_ready || state.done {
                return;
            }
            drop(state);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Why the request got no response, once [`Self::wait_for_response_headers`] returned; a source that failed here has nothing to probe.
    pub fn response_error(&self) -> Option<IoError> {
        let (lock, _) = &*self.state;
        let state = lock.blocking_lock();
        if state.response_ready {
            return None;
        }
        let (kind, message) = state.error.as_ref()?;
        Some(IoError::new(*kind, message.clone()))
    }

    pub fn known_total_size(&self) -> Option<u64> {
        let (lock, _) = &*self.state;
        let state = lock.blocking_lock();
        state.total_size
    }

    fn wait_for_buffer_ahead(&self, min_ahead: usize) {
        let (lock, _notify) = &*self.state;
        loop {
            let state = lock.blocking_lock();
            let buffer_len = state.buffer.len() as u64;
            let buffered_ahead = buffer_len.saturating_sub(self.pos) as usize;

            if buffered_ahead >= min_ahead || state.done || state.error.is_some() {
                return;
            }
            drop(state);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

/// A request that got no answer, told apart so playback can say the host is unreachable.
fn request_kind(error: &reqwest::Error) -> ErrorKind {
    if error.is_connect() {
        ErrorKind::NotConnected
    } else if error.is_timeout() {
        ErrorKind::TimedOut
    } else {
        ErrorKind::Other
    }
}

fn status_kind(status: reqwest::StatusCode) -> ErrorKind {
    match status {
        reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::GONE => ErrorKind::NotFound,
        _ => ErrorKind::Other,
    }
}

impl Read for StreamBuffer {
    fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
        if self.pos == 0 {
            self.wait_for_prebuffer();
        }

        {
            let (lock, _) = &*self.state;
            let state = lock.blocking_lock();
            let buffer_len = state.buffer.len() as u64;
            let buffered_ahead = buffer_len.saturating_sub(self.pos) as usize;

            if buffered_ahead < MIN_BUFFER_AHEAD && !state.done && state.error.is_none() {
                drop(state);
                self.wait_for_buffer_ahead(MIN_BUFFER_AHEAD);
            }
        }

        let (lock, _notify) = &*self.state;
        loop {
            let state = lock.blocking_lock();

            if let Some((kind, err)) = &state.error {
                return Err(IoError::new(*kind, err.clone()));
            }

            let current_len = state.buffer.len() as u64;

            if self.pos < current_len {
                let available = (current_len - self.pos) as usize;
                let to_read = min(buf.len(), available);
                buf[0..to_read].copy_from_slice(
                    &state.buffer[self.pos as usize..(self.pos as usize + to_read)],
                );
                self.pos += to_read as u64;
                return Ok(to_read);
            }

            if state.done {
                if let Some((kind, err)) = &state.error {
                    return Err(IoError::new(*kind, err.clone()));
                }
                return Ok(0);
            }

            drop(state);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Seek for StreamBuffer {
    fn seek(&mut self, pos: SeekFrom) -> IoResult<u64> {
        let (lock, _notify) = &*self.state;
        let state = lock.blocking_lock();

        let len = state.buffer.len() as u64;
        let total = state.total_size.unwrap_or(len);

        let new_pos = match pos {
            SeekFrom::Start(p) => p as i64,
            SeekFrom::Current(p) => self.pos as i64 + p,
            SeekFrom::End(p) => total as i64 + p,
        };

        if new_pos < 0 {
            return Err(IoError::new(
                ErrorKind::InvalidInput,
                "Seek to negative position",
            ));
        }

        self.pos = new_pos as u64;
        Ok(self.pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn missing_length_is_known_once_response_headers_arrive() {
        let state = Arc::new((
            Mutex::new(SharedState {
                buffer: Vec::new(),
                done: false,
                error: None,
                total_size: None,
                response_ready: false,
                prebuffer_ready: false,
            }),
            Notify::new(),
        ));
        let stream = StreamBuffer {
            state: state.clone(),
            pos: 0,
        };
        let (result_tx, result_rx) = mpsc::channel();
        let waiter = std::thread::spawn(move || {
            stream.wait_for_response_headers();
            result_tx
                .send(stream.known_total_size())
                .expect("send header result");
        });

        {
            let (lock, notify) = &*state;
            let mut state = lock.blocking_lock();
            state.response_ready = true;
            notify.notify_waiters();
        }

        let result = result_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("header wait completes without the body finishing");
        waiter.join().expect("join header waiter");

        assert_eq!(result, None);
    }

    /// The extension types of the first ClientHello `client` sends to a local listener that never answers.
    async fn hello_extensions(client: reqwest::Client) -> Vec<u16> {
        use std::io::Read as _;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let request = tokio::spawn(async move {
            let _ = client
                .get(format!("https://127.0.0.1:{port}/"))
                .send()
                .await;
        });
        let record = tokio::task::spawn_blocking(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut header = [0u8; 5];
            stream.read_exact(&mut header).expect("record header");
            let mut body = vec![0u8; u16::from_be_bytes([header[3], header[4]]) as usize];
            stream.read_exact(&mut body).expect("record body");
            body
        })
        .await
        .expect("listener");
        request.abort();
        let u16_at = |at: usize| u16::from_be_bytes([record[at], record[at + 1]]) as usize;
        // Past the handshake header, client version and random, then the session id, suites and compression.
        let mut at = 4 + 2 + 32;
        at += 1 + record[at] as usize;
        at += 2 + u16_at(at);
        at += 1 + record[at] as usize;
        let end = at + 2 + u16_at(at);
        at += 2;
        let mut extensions = Vec::new();
        while at + 4 <= end {
            extensions.push(u16_at(at) as u16);
            at += 4 + u16_at(at + 2);
        }
        extensions
    }

    /// OpenSSL offers encrypt-then-MAC and rustls, having no CBC suites, never does, so the handshake names the backend.
    #[cfg(not(target_os = "android"))]
    #[tokio::test]
    async fn radio_streams_handshake_over_native_tls_and_tracks_over_rustls() {
        const ENCRYPT_THEN_MAC: u16 = 0x0016;
        let radio = hello_extensions(radio_client(None)).await;
        let track = hello_extensions(stream_client(None)).await;
        assert!(
            radio.contains(&ENCRYPT_THEN_MAC),
            "radio offered {radio:x?}"
        );
        assert!(
            !track.contains(&ENCRYPT_THEN_MAC),
            "track offered {track:x?}"
        );
    }
}
