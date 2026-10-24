//! The one seam to a Kopuz daemon: a gRPC client on its own tokio runtime, supervised as a GPUI entity.

mod daemon_process;
mod socket;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::task::Poll;
use std::time::Duration;

use api::{ApiError, ApiEvent, KopuzApi};
use futures::StreamExt as _;
use gpui_kit::{App, AppContext as _, Context, Entity, EventEmitter, Global, Task};

pub use api;
pub use client::Address;
pub use daemon_process::spawn_daemon;
pub use socket::default_socket_path;

const RETRY_INTERVAL: Duration = Duration::from_secs(2);

/// The daemon client plus the runtime its futures run on. Cheap to clone.
#[derive(Clone)]
pub struct Link {
    api: Arc<dyn KopuzApi>,
    runtime: tokio::runtime::Handle,
    address: Address,
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

    /// Run `f` on the link's runtime, awaitable from a GPUI task; until the daemon is attached it fails saying why not.
    pub fn call<T, Fut>(&self, f: impl FnOnce(Arc<dyn KopuzApi>) -> Fut) -> Call<T>
    where
        T: Send + 'static,
        Fut: Future<Output = Result<T, ApiError>> + Send + 'static,
    {
        match self.gate.refusal() {
            Some(error) => Call(self.runtime.spawn(async { Err(error) })),
            None => self.probe(f),
        }
    }

    /// The supervisor's own calls, which are how the gate learns it may open.
    fn probe<T, Fut>(&self, f: impl FnOnce(Arc<dyn KopuzApi>) -> Fut) -> Call<T>
    where
        T: Send + 'static,
        Fut: Future<Output = Result<T, ApiError>> + Send + 'static,
    {
        Call(self.runtime.spawn(f(self.api.clone())))
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

/// Connect to the daemon at `address` and start supervising it; call once before any window opens.
pub fn init(address: Address, cx: &mut App) -> anyhow::Result<()> {
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
    let link = Link {
        api,
        runtime: runtime.handle().clone(),
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
                loop {
                    let probe = link.probe(|api| async move { api.player_state().await });
                    if let Err(error) = probe.await {
                        let unreachable = ConnectionStatus::Unreachable(error);
                        if this
                            .update(cx, |this, cx| {
                                this.attempts = this.attempts.saturating_add(1);
                                this.set_status(unreachable, cx);
                                cx.notify();
                            })
                            .is_err()
                        {
                            return;
                        }
                        let timer = cx.background_executor().timer(RETRY_INTERVAL);
                        // "Retry now" is the same wait, cut short.
                        futures::future::select(Box::pin(timer), Box::pin(retries.next())).await;
                        continue;
                    }

                    // A daemon from another checkout reads fields by its own numbers, so a
                    // mismatch is said out loud rather than left to show as a missing button.
                    let spoken = link
                        .probe(|api| async move { api.daemon_status().await })
                        .await
                        .map(|status| status.proto_revision)
                        .unwrap_or(api::WIRE_REVISION);
                    if spoken != api::WIRE_REVISION {
                        let wrong = ConnectionStatus::Mismatched {
                            daemon: spoken,
                            client: api::WIRE_REVISION,
                        };
                        if this
                            .update(cx, |this, cx| {
                                this.set_status(wrong, cx);
                                cx.notify();
                            })
                            .is_err()
                        {
                            return;
                        }
                        let timer = cx.background_executor().timer(RETRY_INTERVAL);
                        futures::future::select(Box::pin(timer), Box::pin(retries.next())).await;
                        continue;
                    }

                    let mut events = relay_events(&link);
                    let attached = this.update(cx, |this, cx| {
                        this.attempts = 0;
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
