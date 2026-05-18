use std::cmp::min;
use std::io::{Error as IoError, ErrorKind, Read, Result as IoResult, Seek, SeekFrom};
use std::sync::{Arc, Condvar, Mutex};

const MIN_PREBUFFER_BYTES: usize = 256 * 1024; // 256KB

const MIN_BUFFER_AHEAD: usize = 128 * 1024; // 128KB

const MAX_BUFFER_SIZE: usize = 1024 * 1024 * 1024; // 1GB

struct SharedState {
    buffer: Vec<u8>,
    done: bool,
    error: Option<String>,
    total_size: Option<u64>,
    prebuffer_ready: bool,
}

pub struct StreamBuffer {
    state: Arc<(Mutex<SharedState>, Condvar)>,
    pos: u64,
}

impl StreamBuffer {
    pub fn new(url: String, is_radio: bool) -> Self {
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
            Condvar::new(),
        ));

        let state_clone = state.clone();

        let handle = tokio::runtime::Handle::current();
        handle.spawn(async move {
            let client = reqwest::Client::builder()
                .tcp_nodelay(true)
                .user_agent(concat!("Kopuz/", env!("CARGO_PKG_VERSION")))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new());

            match client.get(&url).send().await {
                Ok(mut response) => {
                    if !response.status().is_success() {
                        let (lock, cvar) = &*state_clone;
                        let mut state = lock.lock().unwrap();
                        state.error = Some(format!("HTTP {}", response.status()));
                        state.done = true;
                        state.prebuffer_ready = true;
                        cvar.notify_all();
                        return;
                    }

                    let total_size = response.content_length();
                    {
                        let (lock, cvar) = &*state_clone;
                        let mut state = lock.lock().unwrap();
                        state.total_size = total_size;
                        cvar.notify_all();
                    }

                    let mut total_buffered = 0usize;

                    while let Ok(Some(chunk)) = response.chunk().await {
                        if Arc::strong_count(&state_clone) == 1 {
                            break;
                        }
                        let chunk_len = chunk.len();

                        if total_buffered + chunk_len > MAX_BUFFER_SIZE {
                            let (lock, cvar) = &*state_clone;
                            let mut state = lock.lock().unwrap();
                            state.error = Some("Buffer limit exceeded (1GB)".to_string());
                            state.done = true;
                            state.prebuffer_ready = true;
                            cvar.notify_all();
                            break;
                        }

                        {
                            let (lock, cvar) = &*state_clone;
                            let mut state = lock.lock().unwrap();
                            state.buffer.extend_from_slice(&chunk);
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

                            cvar.notify_all();
                        }
                    }

                    let (lock, cvar) = &*state_clone;
                    let mut state = lock.lock().unwrap();
                    state.done = true;
                    state.prebuffer_ready = true;
                    cvar.notify_all();
                }
                Err(e) => {
                    let (lock, cvar) = &*state_clone;
                    let mut state = lock.lock().unwrap();
                    state.error = Some(e.to_string());
                    state.done = true;
                    state.prebuffer_ready = true;
                    cvar.notify_all();
                }
            }
        });

        Self { state, pos: 0 }
    }

    fn wait_for_prebuffer(&self) {
        let (lock, cvar) = &*self.state;
        let mut state = lock.lock().unwrap();

        while !state.prebuffer_ready && !state.done {
            state = cvar.wait(state).unwrap();
        }
    }

    pub fn wait_for_total_size(&self) {
        let (lock, cvar) = &*self.state;
        let mut state = lock.lock().unwrap();
        while state.total_size.is_none() && !state.done {
            state = cvar.wait(state).unwrap();
        }
    }

    pub fn known_total_size(&self) -> Option<u64> {
        let (lock, _) = &*self.state;
        let state = lock.lock().unwrap();
        state.total_size.or(Some(state.buffer.len() as u64))
    }

    fn wait_for_buffer_ahead(&self, min_ahead: usize) {
        let (lock, cvar) = &*self.state;
        let mut state = lock.lock().unwrap();

        loop {
            let buffer_len = state.buffer.len() as u64;
            let buffered_ahead = buffer_len.saturating_sub(self.pos) as usize;

            if buffered_ahead >= min_ahead || state.done || state.error.is_some() {
                return;
            }

            state = cvar.wait(state).unwrap();
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
            let state = lock.lock().unwrap();
            let buffer_len = state.buffer.len() as u64;
            let buffered_ahead = buffer_len.saturating_sub(self.pos) as usize;

            if buffered_ahead < MIN_BUFFER_AHEAD && !state.done && state.error.is_none() {
                drop(state);
                self.wait_for_buffer_ahead(MIN_BUFFER_AHEAD);
            }
        }

        let (lock, cvar) = &*self.state;
        let mut state = lock.lock().unwrap();

        if let Some(err) = &state.error {
            return Err(IoError::new(ErrorKind::Other, err.clone()));
        }

        loop {
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
                    return Err(IoError::new(ErrorKind::Other, err.clone()));
                }
                return Ok(0);
            }

            state = cvar.wait(state).unwrap();

            if let Some(err) = &state.error {
                return Err(IoError::new(ErrorKind::Other, err.clone()));
            }
        }
    }
}

impl Seek for StreamBuffer {
    fn seek(&mut self, pos: SeekFrom) -> IoResult<u64> {
        let (lock, _cvar) = &*self.state;
        let state = lock.lock().unwrap();

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
