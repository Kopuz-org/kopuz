//! The kopuz side. The screen talks to one `Arc<dyn KopuzApi>`, whichever
//! backend it is: the daemon core hosted in this process ([`crate::inproc`],
//! the default) or a kopuzd over TCP (`client::GrpcApi`, when the system
//! property `debug.kopuz.spike.addr` names an address).
//!
//! A supervisor task on the spike's tokio runtime owns the connection: it
//! connects, loads the tracks (seeding test tones into an empty in-process
//! library), and on failure retries forever with backoff (1, 2, 4, 8, then
//! every 15 s). Results go back to the GPUI thread as [`Msg`] over an
//! unbounded futures channel; nothing here blocks the UI.

use std::fmt;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use api::{
    ApiError, ErrorCode, KopuzApi, Page, Phase, PlayerCommand, PlayerState, QueueContext,
    QueueMode, SetQueueRequest, TrackFilter,
};
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use tokio::runtime::Handle;
use tokio::sync::Notify;

/// The system property that switches the spike to a kopuzd over TCP.
pub const ADDR_PROPERTY: &str = "debug.kopuz.spike.addr";

const PAGE_LIMIT: u32 = 500;
/// Seconds to wait before retry n (1-based); the last entry repeats forever.
const BACKOFF: [u64; 5] = [1, 2, 4, 8, 15];
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const CALL_TIMEOUT: Duration = Duration::from_secs(10);
/// Longest reason shown on the status line.
const REASON_CHARS: usize = 60;

/// Which daemon the screen talks to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Backend {
    /// The daemon core, started inside this process.
    InProcess,
    /// A kopuzd serving gRPC on this address (`kopuzd --listen`).
    Tcp(String),
}

impl Backend {
    /// `debug.kopuz.spike.addr` set and not empty means TCP to that address;
    /// otherwise the core runs in-process. Off Android the environment
    /// variable `KOPUZ_SPIKE_ADDR` stands in for the property.
    pub fn from_environment() -> Self {
        match configured_addr() {
            Some(addr) => Self::Tcp(addr),
            None => Self::InProcess,
        }
    }
}

impl fmt::Display for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InProcess => f.write_str("in-process"),
            Self::Tcp(addr) => write!(f, "tcp addr={addr}"),
        }
    }
}

#[cfg(target_os = "android")]
fn configured_addr() -> Option<String> {
    system_property(ADDR_PROPERTY)
}

#[cfg(not(target_os = "android"))]
fn configured_addr() -> Option<String> {
    std::env::var("KOPUZ_SPIKE_ADDR")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// An Android system property, if it is set and not blank.
#[cfg(target_os = "android")]
fn system_property(name: &str) -> Option<String> {
    // PROP_VALUE_MAX in <sys/system_properties.h>, including the NUL.
    const PROP_VALUE_MAX: usize = 92;
    let name = std::ffi::CString::new(name).ok()?;
    let mut value = [0 as libc::c_char; PROP_VALUE_MAX];
    // SAFETY: `name` is NUL-terminated and `value` has the PROP_VALUE_MAX
    // bytes the call may write, NUL included.
    let len = unsafe { libc::__system_property_get(name.as_ptr(), value.as_mut_ptr()) };
    if len <= 0 {
        return None;
    }
    // SAFETY: the call NUL-terminated `value`.
    let value = unsafe { std::ffi::CStr::from_ptr(value.as_ptr()) };
    let value = value.to_string_lossy().trim().to_string();
    (!value.is_empty()).then_some(value)
}

/// One list row.
#[derive(Debug, Clone)]
pub struct Row {
    pub key: String,
    pub title: String,
    pub artist: String,
}

/// Where the connection is; the status line is drawn from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnState {
    /// The first attempt is under way.
    Connecting,
    /// A later attempt is under way.
    Reconnecting,
    /// Waiting `secs` more seconds before the next attempt. `reason` is why
    /// the last attempt failed; `None` when a working connection was lost.
    Waiting { reason: Option<String>, secs: u64 },
    /// The daemon answers.
    Connected,
}

impl ConnState {
    fn kind(&self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Reconnecting | Self::Waiting { reason: None, .. } => "reconnecting",
            Self::Waiting {
                reason: Some(_), ..
            } => "unreachable",
            Self::Connected => "connected",
        }
    }
}

/// What the runtime reports back to the UI.
#[derive(Debug)]
pub enum Msg {
    State(ConnState),
    Tracks(Vec<Row>),
    Playing(bool),
    /// A line shown after the track count (seeding, a failed call), or `None` to clear it.
    Note(Option<String>),
}

/// Start a multi-thread tokio runtime owned by a background thread and
/// return its handle. The thread parks on the runtime until the process ends.
pub fn start_runtime() -> std::io::Result<Handle> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("kopuz-spike-rt".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("kopuz-spike-io")
                .enable_all()
                .build();
            match runtime {
                Ok(runtime) => {
                    let _ = tx.send(Ok(runtime.handle().clone()));
                    runtime.block_on(futures::future::pending::<()>());
                }
                Err(error) => {
                    let _ = tx.send(Err(error));
                }
            }
        })?;
    rx.recv()
        .map_err(|_| std::io::Error::other("tokio runtime thread exited"))?
}

fn is_playing(phase: Phase) -> bool {
    phase == Phase::Playing
}

/// The daemon went away, as opposed to answering with an error.
fn is_connection_error(error: &ApiError) -> bool {
    error.code == ErrorCode::DaemonGone
}

/// The part of an error worth a status line: its message, first line, capped.
fn short_reason(error: &ApiError) -> String {
    let line = error.message.lines().next().unwrap_or_default().trim();
    let line = if line.is_empty() {
        format!("{:?}", error.code)
    } else {
        line.to_string()
    };
    if line.chars().count() > REASON_CHARS {
        let cut: String = line.chars().take(REASON_CHARS - 1).collect();
        format!("{cut}…")
    } else {
        line
    }
}

/// A call that failed: the daemon is gone (or did not answer in time), or it answered with an error.
enum CallError {
    Lost(String),
    Failed(String),
}

async fn call<T>(future: impl Future<Output = Result<T, ApiError>>) -> Result<T, CallError> {
    match tokio::time::timeout(CALL_TIMEOUT, future).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) if is_connection_error(&error) => Err(CallError::Lost(short_reason(&error))),
        Ok(Err(error)) => Err(CallError::Failed(short_reason(&error))),
        Err(_) => Err(CallError::Lost("timed out".into())),
    }
}

/// State the supervisor and the UI's calls share.
struct Shared {
    tx: UnboundedSender<Msg>,
    /// The connected API, while there is one.
    api: Mutex<Option<Arc<dyn KopuzApi>>>,
    /// Woken when a call finds the daemon gone.
    lost: Notify,
    /// The kind of the last state logged, so each change is logged once.
    logged: Mutex<&'static str>,
}

impl Shared {
    fn send(&self, msg: Msg) {
        let _ = self.tx.unbounded_send(msg);
    }

    fn set_state(&self, state: ConnState) {
        let kind = state.kind();
        let mut logged = self.logged.lock().unwrap_or_else(|poison| poison.into_inner());
        if *logged != kind {
            *logged = kind;
            match &state {
                ConnState::Waiting {
                    reason: Some(reason),
                    secs,
                } => log::info!("SPIKE state={kind} reason={reason} retry_in={secs}s"),
                _ => log::info!("SPIKE state={kind}"),
            }
        }
        drop(logged);
        self.send(Msg::State(state));
    }

    fn api(&self) -> Option<Arc<dyn KopuzApi>> {
        self.api
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    fn set_api(&self, api: Option<Arc<dyn KopuzApi>>) {
        *self.api.lock().unwrap_or_else(|poison| poison.into_inner()) = api;
    }

    /// A call found the daemon gone: drop the API and wake the supervisor.
    fn connection_lost(&self, what: &str, reason: &str) {
        log::info!("SPIKE error {what}: connection lost: {reason}");
        self.set_api(None);
        self.lost.notify_waiters();
    }

    /// A call the daemon answered with an error.
    fn call_failed(&self, what: &str, reason: String) {
        log::info!("SPIKE error {what}: {reason}");
        self.send(Msg::Note(Some(format!("{what} failed: {reason}"))));
    }
}

pub struct Daemon {
    handle: Handle,
    backend: Backend,
    shared: Arc<Shared>,
}

impl Daemon {
    /// Nothing is dialled or started until [`Daemon::start`].
    pub fn new(handle: Handle, backend: Backend) -> (Self, UnboundedReceiver<Msg>) {
        let (tx, rx) = unbounded();
        let shared = Arc::new(Shared {
            tx,
            api: Mutex::new(None),
            lost: Notify::new(),
            logged: Mutex::new(""),
        });
        (
            Self {
                handle,
                backend,
                shared,
            },
            rx,
        )
    }

    /// Start the supervisor: connect, load the tracks, and keep reconnecting.
    pub fn start(&self) {
        let shared = Arc::clone(&self.shared);
        let backend = self.backend.clone();
        self.handle.spawn(supervise(shared, backend));
    }

    /// Run `f` against the connected API on the runtime; without a
    /// connection there is nothing to do (the status line already says so).
    fn spawn<F, Fut>(&self, what: &'static str, f: F)
    where
        F: FnOnce(Arc<dyn KopuzApi>, Arc<Shared>) -> Fut,
        Fut: Future<Output = ()> + Send + 'static,
    {
        match self.shared.api() {
            Some(api) => {
                self.handle.spawn(f(api, Arc::clone(&self.shared)));
            }
            None => log::info!("SPIKE error {what}: not connected"),
        }
    }

    /// Replace the queue with `keys` and start at `index`.
    pub fn play(&self, keys: Vec<String>, index: usize) {
        self.spawn("play", move |api, shared| async move {
            let request = SetQueueRequest {
                mode: QueueMode::Replace,
                context: QueueContext::Tracks { keys },
                start_index: Some(index as u32),
                shuffle: None,
            };
            match call(api.set_queue(request)).await {
                Ok(_) => {
                    log::info!("SPIKE play ok index={index}");
                    shared.send(Msg::Note(None));
                    shared.send(Msg::Playing(true));
                }
                Err(CallError::Lost(reason)) => shared.connection_lost("play", &reason),
                Err(CallError::Failed(reason)) => shared.call_failed("Play", reason),
            }
        });
    }

    /// Play/pause, then re-read the phase so the button shows the truth.
    pub fn toggle(&self) {
        self.spawn("toggle", |api, shared| async move {
            match call(api.player_command(PlayerCommand::Toggle)).await {
                Ok(_) => {
                    log::info!("SPIKE toggle ok");
                    shared.send(Msg::Note(None));
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    match call(api.player_state()).await {
                        Ok(state) => shared.send(Msg::Playing(is_playing(state.phase))),
                        Err(CallError::Lost(reason)) => shared.connection_lost("toggle", &reason),
                        Err(CallError::Failed(_)) => {}
                    }
                }
                Err(CallError::Lost(reason)) => shared.connection_lost("toggle", &reason),
                Err(CallError::Failed(reason)) => shared.call_failed("Play/pause", reason),
            }
        });
    }
}

/// Seconds before retry `failures` (1-based).
fn backoff(failures: u32) -> u64 {
    let index = (failures.max(1) as usize - 1).min(BACKOFF.len() - 1);
    BACKOFF[index]
}

/// Count down to the next attempt, one state update per second.
async fn wait_to_retry(shared: &Shared, reason: Option<String>, secs: u64) {
    for left in (1..=secs).rev() {
        shared.set_state(ConnState::Waiting {
            reason: reason.clone(),
            secs: left,
        });
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// Reach the backend and prove it answers.
async fn connect(
    backend: &Backend,
    tcp: &mut Option<Arc<dyn KopuzApi>>,
) -> Result<(Arc<dyn KopuzApi>, PlayerState), String> {
    let api = match backend {
        // Blocks until the core has opened its library, so off the async workers.
        Backend::InProcess => tokio::task::spawn_blocking(crate::inproc::start)
            .await
            .map_err(|error| format!("core start: {error}"))??,
        // One lazy tonic channel for the life of the app; it redials by itself.
        Backend::Tcp(addr) => match tcp {
            Some(api) => Arc::clone(api),
            None => {
                let api: Arc<dyn KopuzApi> = Arc::new(
                    client::GrpcApi::connect_tcp(addr.as_str())
                        .map_err(|error| short_reason(&error))?,
                );
                *tcp = Some(Arc::clone(&api));
                api
            }
        },
    };
    match tokio::time::timeout(PROBE_TIMEOUT, api.player_state()).await {
        Ok(Ok(state)) => Ok((api, state)),
        Ok(Err(error)) => Err(short_reason(&error)),
        Err(_) => Err("timed out".into()),
    }
}

/// Connect, load, wait for the connection to be lost, and start over; forever.
async fn supervise(shared: Arc<Shared>, backend: Backend) {
    let mut tcp = None;
    let mut failures = 0u32;
    shared.set_state(ConnState::Connecting);
    loop {
        match connect(&backend, &mut tcp).await {
            Ok((api, player)) => {
                failures = 0;
                match &backend {
                    Backend::InProcess => log::info!("SPIKE connected backend=in-process"),
                    Backend::Tcp(addr) => log::info!("SPIKE connected addr={addr}"),
                }
                // Listen before the API is handed out, so no loss is missed.
                let lost = shared.lost.notified();
                tokio::pin!(lost);
                lost.as_mut().enable();
                shared.set_api(Some(Arc::clone(&api)));
                shared.set_state(ConnState::Connected);
                shared.send(Msg::Playing(is_playing(player.phase)));
                match load_tracks(&shared, &backend, api.as_ref()).await {
                    Ok(()) => lost.await,
                    Err(reason) => shared.connection_lost("tracks", &reason),
                }
                shared.set_api(None);
                wait_to_retry(&shared, None, backoff(1)).await;
            }
            Err(reason) => {
                failures += 1;
                log::info!("SPIKE error connect {backend} try {failures}: {reason}");
                wait_to_retry(&shared, Some(reason), backoff(failures)).await;
            }
        }
        shared.set_state(ConnState::Reconnecting);
    }
}

/// Load the first page of tracks; an empty in-process library is seeded
/// with test tones first. `Err` only when the connection was lost.
async fn load_tracks(shared: &Shared, backend: &Backend, api: &dyn KopuzApi) -> Result<(), String> {
    let page = Page {
        offset: 0,
        limit: PAGE_LIMIT,
    };
    let mut tracks = match call(api.tracks(TrackFilter::default(), page)).await {
        Ok(tracks) => tracks,
        Err(CallError::Lost(reason)) => return Err(reason),
        Err(CallError::Failed(reason)) => {
            shared.call_failed("Tracks", reason);
            return Ok(());
        }
    };
    if tracks.total == 0 && *backend == Backend::InProcess {
        shared.send(Msg::Note(Some("Adding test tones…".into())));
        match crate::seed::seed(api).await {
            Ok(n) => {
                log::info!("SPIKE seeded n={n}");
                shared.send(Msg::Note(None));
            }
            Err(error) => shared.call_failed("Seeding", short_reason(&error)),
        }
        tracks = match call(api.tracks(TrackFilter::default(), page)).await {
            Ok(tracks) => tracks,
            Err(CallError::Lost(reason)) => return Err(reason),
            Err(CallError::Failed(reason)) => {
                shared.call_failed("Tracks", reason);
                return Ok(());
            }
        };
    }
    let rows: Vec<Row> = tracks
        .items
        .into_iter()
        .map(|t| Row {
            key: t.key,
            title: t.title,
            artist: t.artist,
        })
        .collect();
    log::info!("SPIKE tracks loaded n={}", rows.len());
    shared.send(Msg::Tracks(rows));
    Ok(())
}
