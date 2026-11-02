//! The one seam to a Kopuz daemon: a gRPC client on its own tokio runtime, supervised as a GPUI entity.

mod daemon_process;
mod replace;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::task::Poll;
use std::time::Duration;
use std::time::Instant;

use api::{ApiError, ApiEvent, KopuzApi};
use futures::StreamExt as _;
use gpui_kit::{App, AppContext as _, AsyncApp, Context, Entity, EventEmitter, Global, Task};

pub use api;
pub use client::Address;
pub use client::default_socket_path;
pub use daemon_process::run_daemon_flag;

const RETRY_INTERVAL: Duration = Duration::from_secs(2);
/// How many times the app starts its own kopuzd before giving up and crashing.
const START_TRIES: u32 = 3;
/// A kopuzd still alive but silent this long is stuck, not slow.
const START_WAIT: Duration = Duration::from_secs(60);
const START_POLL: Duration = Duration::from_millis(250);
/// A daemon this slow to say what state it is in is wedged, however it holds the socket.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// The daemon client plus the runtime its futures run on. Cheap to clone.
#[derive(Clone)]
pub struct Link {
    api: Arc<dyn KopuzApi>,
    runtime: tokio::runtime::Handle,
    address: Address,
    /// The socket this app starts its own kopuzd on when nothing answers; `None` for a daemon run elsewhere.
    own_socket: Option<PathBuf>,
    /// Open only while the connection is `Connected`; a daemon on another wire would misread every call.
    gate: Arc<Gate>,
}

/// Why calls may not reach the daemon right now, as the supervisor last found it.
struct Gate(Mutex<Option<ApiError>>);

impl Gate {
    fn closed() -> Self {
        let gate = Self(Mutex::new(None));
        gate.follow(&ConnectionStatus::Connecting);
        gate
    }

    fn refusal(&self) -> Option<ApiError> {
        self.0
            .lock()
            .unwrap_or_else(|held| held.into_inner())
            .clone()
    }

    fn follow(&self, status: &ConnectionStatus) {
        let refusal = match status {
            ConnectionStatus::Connected => None,
            ConnectionStatus::Mismatched { daemon, client } => {
                Some(protocol_mismatch(*daemon, *client))
            }
            ConnectionStatus::Connecting | ConnectionStatus::Unreachable(_) => Some(ApiError::new(
                api::ErrorCode::DaemonGone,
                "the daemon is not attached",
            )),
        };
        *self.0.lock().unwrap_or_else(|held| held.into_inner()) = refusal;
    }
}

/// What a call to a daemon on another wire fails with, worded as `kopuzctl` words it.
fn protocol_mismatch(daemon: u32, client: u32) -> ApiError {
    ApiError::unsupported(format!(
        "protocol mismatch: daemon {daemon}, client {client}; install matching Kopuz versions"
    ))
}

struct GlobalLink {
    link: Link,
    connection: Entity<Connection>,
    // Dropping the runtime would cancel every call in flight.
    _runtime: Arc<tokio::runtime::Runtime>,
}

impl Global for GlobalLink {}

impl Link {
    /// The link for the running app; panics before [`init`].
    pub fn global(cx: &App) -> &Link {
        &cx.global::<GlobalLink>().link
    }

    pub fn connection(cx: &App) -> Entity<Connection> {
        cx.global::<GlobalLink>().connection.clone()
    }

    pub fn address(&self) -> &Address {
        &self.address
    }

    /// Whether this app starts its own kopuzd when nothing answers.
    pub fn starts_own(&self) -> bool {
        self.own_socket.is_some()
    }

    async fn answers(&self) -> bool {
        self.probe(|api| async move { api.player_state().await })
            .await
            .is_ok()
    }

    /// Run `f` on the link's runtime, awaitable from a GPUI task; until the daemon is attached it fails saying why not.
    pub fn call<T, Fut>(&self, f: impl FnOnce(Arc<dyn KopuzApi>) -> Fut) -> Call<T>
    where
        T: Send + 'static,
        Fut: Future<Output = Result<T, ApiError>> + Send + 'static,
    {
        match self.gate.refusal() {
            Some(error) => Call(self.runtime.spawn(async { Err(error) })),
            None => Call(self.runtime.spawn(f(self.api.clone()))),
        }
    }

    /// The supervisor's own calls, which are how the gate learns it may open; a peer that never replies counts as gone.
    fn probe<T, Fut>(&self, f: impl FnOnce(Arc<dyn KopuzApi>) -> Fut) -> Call<T>
    where
        T: Send + 'static,
        Fut: Future<Output = Result<T, ApiError>> + Send + 'static,
    {
        let call = f(self.api.clone());
        Call(self.runtime.spawn(async move {
            tokio::time::timeout(PROBE_TIMEOUT, call)
                .await
                .unwrap_or_else(|_| {
                    Err(ApiError::new(
                        api::ErrorCode::DaemonGone,
                        format!(
                            "the daemon did not answer within {} seconds",
                            PROBE_TIMEOUT.as_secs()
                        ),
                    ))
                })
        }))
    }

    /// Fire a command whose effect arrives on the event stream; failure is only logged.
    pub fn fire<T, Fut>(
        &self,
        cx: &App,
        what: &'static str,
        f: impl FnOnce(Arc<dyn KopuzApi>) -> Fut,
    ) where
        T: Send + 'static,
        Fut: Future<Output = Result<T, ApiError>> + Send + 'static,
    {
        let call = self.call(f);
        cx.background_spawn(async move {
            if let Err(error) = call.await {
                tracing::warn!(%error, what, "daemon command failed");
            }
        })
        .detach();
    }
}

/// A daemon call in flight on the link's runtime.
pub struct Call<T>(tokio::task::JoinHandle<Result<T, ApiError>>);

impl<T> Future for Call<T> {
    type Output = Result<T, ApiError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.0).poll(cx).map(|joined| {
            joined.unwrap_or_else(|error| {
                Err(ApiError::internal(format!("daemon call aborted: {error}")))
            })
        })
    }
}

/// Connect to and supervise the daemon at `address`, before any window opens; `start_own` starts this app's own kopuzd on a free socket.
pub fn init(address: Address, start_own: bool, cx: &mut App) -> anyhow::Result<()> {
    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("kopuz-link")
            .enable_all()
            .build()?,
    );
    let api: Arc<dyn KopuzApi> = {
        let _guard = runtime.enter();
        match &address {
            Address::Socket(path) => Arc::new(client::GrpcApi::new(path.clone())?),
            Address::Tcp(host) => Arc::new(client::GrpcApi::connect_tcp(host.clone())?),
        }
    };
    let own_socket = match (&address, start_own) {
        (Address::Socket(path), true) => Some(path.clone()),
        _ => None,
    };
    let link = Link {
        api,
        runtime: runtime.handle().clone(),
        own_socket,
        address,
        gate: Arc::new(Gate::closed()),
    };
    let connection = cx.new(|cx| Connection::new(link.clone(), cx));
    cx.set_global(GlobalLink {
        link,
        connection,
        _runtime: runtime,
    });
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionStatus {
    Connecting,
    Connected,
    /// The daemon answers, but not in the wire this build was compiled against.
    /// Going on would read every field by the numbers the other side gave them.
    Mismatched {
        daemon: u32,
        client: u32,
    },
    /// The last attempt failed; another follows after [`RETRY_INTERVAL`].
    Unreachable(ApiError),
}

/// Supervises the daemon socket and relays its event stream.
pub struct Connection {
    status: ConnectionStatus,
    /// Failed probes since the last time the daemon answered, for the banner's "attempt 3".
    attempts: u32,
    /// This app is starting its own kopuzd, which either answers or crashes the app; retrying cannot help.
    starting: bool,
    /// The own socket is held by something that does not answer, so starting a kopuzd there would only fail to bind.
    blocked: bool,
    retry: futures::channel::mpsc::UnboundedSender<()>,
    link: Link,
    _supervisor: Task<()>,
}

impl EventEmitter<ApiEvent> for Connection {}

impl Connection {
    fn new(link: Link, cx: &mut Context<Self>) -> Self {
        let (retry, mut retries) = futures::channel::mpsc::unbounded();
        let supervisor = cx.spawn({
            let link = link.clone();
            async move |this, cx| {
                // Once per run: a second older daemon means the kopuzd beside the app is the old one.
                let mut replaced = false;
                loop {
                    let probe = link.probe(|api| async move { api.player_state().await });
                    if let Err(error) = probe.await {
                        let own = link.own_socket.clone();
                        let blocked = own.as_deref().is_some_and(socket_taken);
                        let start = own.filter(|_| !blocked);
                        let unreachable = ConnectionStatus::Unreachable(error);
                        if this
                            .update(cx, |this, cx| {
                                this.attempts = this.attempts.saturating_add(1);
                                this.starting = start.is_some();
                                this.blocked = blocked;
                                this.set_status(unreachable, cx);
                                cx.notify();
                            })
                            .is_err()
                        {
                            return;
                        }
                        if let Some(socket) = start {
                            start_daemon(&link, &socket, cx).await;
                            continue;
                        }
                        wait_or_retry(&mut retries, cx).await;
                        continue;
                    }

                    let status = link
                        .probe(|api| async move { api.daemon_status().await })
                        .await
                        .ok();
                    let spoken = status
                        .as_ref()
                        .map_or(api::WIRE_REVISION, |status| status.proto_revision);
                    let outdated = status.as_ref().is_some_and(|status| {
                        status.proto_revision < api::WIRE_REVISION
                            || replace::is_older(&status.version)
                    });
                    if let Some(socket) = link.own_socket.clone().filter(|_| outdated && !replaced)
                    {
                        replaced = true;
                        let starting = this.update(cx, |this, cx| {
                            this.starting = true;
                            this.set_status(ConnectionStatus::Connecting, cx);
                            cx.notify();
                        });
                        if starting.is_err() {
                            return;
                        }
                        if replace::stop_daemon(&socket, cx).await {
                            continue;
                        }
                    }

                    // A daemon from another checkout reads fields by its own numbers, so a
                    // mismatch is said out loud rather than left to show as a missing button.
                    if spoken != api::WIRE_REVISION {
                        let wrong = ConnectionStatus::Mismatched {
                            daemon: spoken,
                            client: api::WIRE_REVISION,
                        };
                        if this
                            .update(cx, |this, cx| {
                                this.starting = false;
                                this.blocked = false;
                                this.set_status(wrong, cx);
                                cx.notify();
                            })
                            .is_err()
                        {
                            return;
                        }
                        wait_or_retry(&mut retries, cx).await;
                        continue;
                    }

                    let mut events = relay_events(&link);
                    let attached = this.update(cx, |this, cx| {
                        this.attempts = 0;
                        this.starting = false;
                        this.blocked = false;
                        this.set_status(ConnectionStatus::Connected, cx);
                        cx.emit(ApiEvent::Resync);
                    });
                    if attached.is_err() {
                        return;
                    }
                    while let Some(event) = events.next().await {
                        if this.update(cx, |_, cx| cx.emit(event)).is_err() {
                            return;
                        }
                    }
                    tracing::info!("daemon event stream ended");
                    let lost = ConnectionStatus::Connecting;
                    if this
                        .update(cx, |this, cx| this.set_status(lost, cx))
                        .is_err()
                    {
                        return;
                    }
                }
            }
        });
        Self {
            status: ConnectionStatus::Connecting,
            attempts: 0,
            starting: false,
            blocked: false,
            retry,
            link,
            _supervisor: supervisor,
        }
    }

    pub fn status(&self) -> &ConnectionStatus {
        &self.status
    }

    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    /// Whether this app is starting its own kopuzd right now.
    pub fn is_starting(&self) -> bool {
        self.starting
    }

    /// Whether the own socket is held by a daemon that does not answer, which only someone outside the app can clear.
    pub fn is_blocked(&self) -> bool {
        self.blocked
    }

    /// Stop waiting out the retry interval and probe the daemon again now.
    pub fn retry_now(&self) {
        let _ = self.retry.unbounded_send(());
    }

    pub fn is_connected(&self) -> bool {
        self.status == ConnectionStatus::Connected
    }

    pub fn link(&self) -> &Link {
        &self.link
    }

    fn set_status(&mut self, status: ConnectionStatus, cx: &mut Context<Self>) {
        self.link.gate.follow(&status);
        if self.status != status {
            self.status = status;
            cx.notify();
        }
    }
}

/// Start kopuzd until it answers, up to [`START_TRIES`] times, then panic so the crash report says why.
async fn start_daemon(link: &Link, socket: &Path, cx: &mut AsyncApp) {
    let mut reason = String::new();
    for attempt in 1..=START_TRIES {
        if attempt > 1 {
            cx.background_executor().timer(RETRY_INTERVAL).await;
        }
        // Another app or a service manager may have started one meanwhile.
        if link.answers().await {
            return;
        }
        let mut child = match daemon_process::spawn_daemon(socket) {
            Ok(child) => child,
            Err(error) => {
                reason = format!("could not run kopuzd: {error}");
                tracing::warn!(attempt, %reason, "kopuzd did not start");
                continue;
            }
        };
        let spawned = Instant::now();
        loop {
            cx.background_executor().timer(START_POLL).await;
            if link.answers().await {
                tracing::info!(attempt, "started kopuzd");
                return;
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    reason = format!("kopuzd exited with {status}");
                    break;
                }
                // Alive but silent: killing it could cut a migration short, and a second one would only fail to bind.
                Ok(None) if spawned.elapsed() >= START_WAIT => panic!(
                    "kopuzd started but did not answer within {} seconds",
                    START_WAIT.as_secs()
                ),
                Ok(None) => {}
                Err(error) => {
                    reason = format!("could not watch kopuzd: {error}");
                    break;
                }
            }
        }
        tracing::warn!(attempt, %reason, "kopuzd did not start");
    }
    panic!("kopuzd did not start after {START_TRIES} tries: {reason}");
}

/// Wait out [`RETRY_INTERVAL`], or less if "Retry now" is pressed during it; presses from before the wait do not count.
async fn wait_or_retry(retries: &mut futures::channel::mpsc::UnboundedReceiver<()>, cx: &AsyncApp) {
    while retries.try_recv().is_ok() {}
    let timer = cx.background_executor().timer(RETRY_INTERVAL);
    futures::future::select(Box::pin(timer), Box::pin(retries.next())).await;
}

/// Something accepts connections on `socket`, so a kopuzd started there could not bind it.
#[cfg(unix)]
fn socket_taken(socket: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(socket).is_ok()
}

/// Opening a named pipe connects to it; a busy one is held all the same.
#[cfg(windows)]
fn socket_taken(pipe: &Path) -> bool {
    const ERROR_PIPE_BUSY: i32 = 231;
    match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(pipe)
    {
        Ok(_) => true,
        Err(error) => error.raw_os_error() == Some(ERROR_PIPE_BUSY),
    }
}

/// Subscribe on the link's runtime and forward into a channel GPUI can poll.
fn relay_events(link: &Link) -> futures::channel::mpsc::UnboundedReceiver<ApiEvent> {
    let (tx, rx) = futures::channel::mpsc::unbounded();
    let api = link.api.clone();
    link.runtime.spawn(async move {
        let mut stream = api.events();
        while let Some(event) = stream.next().await {
            if tx.unbounded_send(event).is_err() {
                return;
            }
        }
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn a_socket_is_taken_only_while_something_listens_on_it() {
        let socket = std::env::temp_dir().join(format!("kopuz-taken-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket);
        assert!(!socket_taken(&socket));
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        assert!(socket_taken(&socket));
        drop(listener);
        // A file left behind by a daemon that died is not a daemon.
        assert!(!socket_taken(&socket));
        std::fs::remove_file(&socket).unwrap();
    }

    /// The banner reads this to say which of the two is the stale one.
    fn older(status: &ConnectionStatus) -> Option<bool> {
        match status {
            ConnectionStatus::Mismatched { daemon, client } => Some(daemon < client),
            _ => None,
        }
    }

    #[test]
    fn a_daemon_speaking_this_wire_is_not_a_mismatch() {
        assert_eq!(api::WIRE_REVISION, api::WIRE_REVISION);
        assert_eq!(older(&ConnectionStatus::Connected), None);
    }

    /// A daemon predating the check answers zero, which is every build before it.
    #[test]
    fn a_silent_daemon_reads_as_the_older_one() {
        let status = ConnectionStatus::Mismatched {
            daemon: 0,
            client: api::WIRE_REVISION,
        };
        assert_eq!(older(&status), Some(true));
    }

    #[test]
    fn the_gate_refuses_with_what_actually_happened() {
        let gate = Gate::closed();
        assert_eq!(
            gate.refusal().map(|error| error.code),
            Some(api::ErrorCode::DaemonGone)
        );
        gate.follow(&ConnectionStatus::Mismatched {
            daemon: 1,
            client: 2,
        });
        let refused = gate.refusal().expect("a mismatch refuses");
        assert_eq!(refused.code, api::ErrorCode::Unsupported);
        assert!(
            refused
                .message
                .starts_with("protocol mismatch: daemon 1, client 2")
        );
        gate.follow(&ConnectionStatus::Connected);
        assert!(gate.refusal().is_none());
    }

    #[test]
    fn a_frontend_left_behind_says_so_the_other_way() {
        let status = ConnectionStatus::Mismatched {
            daemon: api::WIRE_REVISION + 1,
            client: api::WIRE_REVISION,
        };
        assert_eq!(older(&status), Some(false));
    }
}
