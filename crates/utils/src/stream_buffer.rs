use std::cmp::min;
use std::io::{Error as IoError, ErrorKind, Read, Result as IoResult, Seek, SeekFrom};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, Notify};
use tracing::Instrument;

// Architecture:
// - A background download task (tokio::spawn) fetches audio chunks via HTTP
// - Synchronous Read/Seek impls consume the buffer (required by symphonia decoder)
// - Shared state uses tokio::sync::Mutex so the async task never blocks
//   a Tokio worker thread with std::sync::Mutex::lock()
// - The sync side uses blocking_lock() + 5ms polling (acceptable latency for audio)
// - Prebuffering ensures at least 256KB before first read to avoid starvation

const MIN_PREBUFFER_BYTES: usize = 256 * 1024; // 256KB

const MIN_BUFFER_AHEAD: usize = 128 * 1024; // 128KB

const MAX_BUFFER_SIZE: usize = 1024 * 1024 * 1024; // 1GB

/// Shared state between the async downloader and sync readers.
///
/// Fields:
/// - `buffer`: accumulated audio bytes
/// - `done`: HTTP stream finished (success or error)
/// - `error`: reason for failure, if any
/// - `total_size`: Content-Length header value (may be unknown for radio/streams)
/// - `prebuffer_ready`: enough data buffered to start playback safely
struct SharedState {
    buffer: Vec<u8>,
    done: bool,
    error: Option<String>,
    total_size: Option<u64>,
    prebuffer_ready: bool,
}

pub struct StreamBuffer {
    state: Arc<(Mutex<SharedState>, Notify)>,
    pos: u64,
}

impl StreamBuffer {
    /// `runtime` is the handle the background download runs on. It's passed in
    /// (not `Handle::current()`) because construction happens on the engine's
    /// decode worker — a plain thread with no runtime in scope. The sync reader
    /// side uses `blocking_lock()` and needs no runtime.
    ///
    /// `icy_tx`: when set, ICY metadata is requested, stripped from the
    /// audio, and published on the channel.
    pub fn with_user_agent(
        url: String,
        is_radio: bool,
        user_agent: Option<String>,
        icy_tx: Option<tokio::sync::watch::Sender<crate::icy::IcyMeta>>,
        runtime: tokio::runtime::Handle,
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
                prebuffer_ready: false,
            }),
            Notify::new(),
        ));

        let state_clone = state.clone();

        // Background download task on the app runtime; shared state via
        // tokio::sync::Mutex::lock().await (never blocks a worker thread).
        runtime.spawn(
            async move {
                let ua = user_agent
                    .unwrap_or_else(|| concat!("Kopuz/", env!("CARGO_PKG_VERSION")).to_string());
                let client = reqwest::Client::builder()
                    .tcp_nodelay(true)
                    .user_agent(ua)
                    .build()
                    .unwrap_or_else(|_| reqwest::Client::new());

                // Radio directories often hand out a playlist (.pls/.m3u par example)
                // instead of the stream; follow it to its first entry.
                let mut url = url;
                let mut hops = 0u8;
                let result = loop {
                    if hops > 3 {
                        break Err("too many playlist redirects".to_string());
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
                            if !crate::playlist::is_playlist(content_type, response.url().path()) {
                                break Ok(response);
                            }
                            match response.text().await.ok().as_deref().and_then(|text| {
                                crate::playlist::first_stream_url(text)
                                    // Playlist 2 playlist is HLS or a loop;
                                    // not decodable, so stop here.
                                    .filter(|next| !crate::playlist::is_playlist(None, next))
                            }) {
                                Some(next) => {
                                    tracing::debug!(from = %url, to = %next, "resolved playlist to stream URL");
                                    url = next;
                                }
                                None => {
                                    break Err("playlist has no playable stream URL".to_string());
                                }
                            }
                        }
                        Ok(response) => break Err(format!("HTTP {}", response.status())),
                        Err(e) => break Err(e.to_string()),
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
                        {
                            let (lock, notify) = &*state_clone;
                            let mut state = lock.lock().await;
                            state.total_size = total_size;
                            notify.notify_waiters();
                        }

                        // De-interleave only if the server honours the
                        // Icy-MetaData request.
                        let mut icy = icy_tx.and_then(|tx| {
                            let metaint = response
                                .headers()
                                .get("icy-metaint")
                                .and_then(|v| v.to_str().ok())
                                .and_then(|v| v.trim().parse::<usize>().ok())
                                .filter(|m| *m > 0)?;
                            tracing::debug!(metaint, "ICY metadata enabled for radio stream");
                            Some((crate::icy::IcyDeinterleaver::new(metaint), tx))
                        });

                        let mut total_buffered = 0usize;
                        let mut audio_scratch = Vec::new();

                        while let Ok(Some(chunk)) = response.chunk().await {
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
                                state.error = Some("Buffer limit exceeded (1GB)".to_string());
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
                        }

                        let (lock, notify) = &*state_clone;
                        let mut state = lock.lock().await;
                        state.done = true;
                        state.prebuffer_ready = true;
                        notify.notify_waiters();
                    }
                    Err(e) => {
                        let (lock, notify) = &*state_clone;
                        let mut state = lock.lock().await;
                        state.error = Some(e.to_string());
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

    // Blocking wait helpers.
    // These are called from the sync Read impl, so they use blocking_lock()
    // on the tokio::sync::Mutex. A 5ms polling interval is negligible compared
    // to network latency (~100ms+) and audio decode times.
    // The async download side uses Notify::notify_waiters() to wake any
    // eventual future blocking_lock waiter faster, though polling alone suffices.

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

    pub fn wait_for_total_size(&self) {
        let (lock, _notify) = &*self.state;
        loop {
            let state = lock.blocking_lock();
            if state.total_size.is_some() || state.done {
                return;
            }
            drop(state);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    pub fn known_total_size(&self) -> Option<u64> {
        let (lock, _) = &*self.state;
        let state = lock.blocking_lock();
        state.total_size.or(Some(state.buffer.len() as u64))
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

        // Main read loop:
        // 1. If data is available at current pos → copy into buf, advance pos, return
        // 2. If download is finished (done) → return 0 (EOF) or error
        // 3. Otherwise → wait for more data from the download task and retry
        let (lock, _notify) = &*self.state;
        loop {
            let state = lock.blocking_lock();

            if let Some(err) = &state.error {
                return Err(IoError::other(err.clone()));
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
                if let Some(err) = &state.error {
                    return Err(IoError::other(err.clone()));
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
