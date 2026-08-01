//! The plugin supervisor: one child process, one handshake, one request map.
//!
//! [`PluginClient`] owns everything about a running plugin — the child, the
//! three stdio pumps, the in-flight request table, the health ping and the
//! restart policy. It is a cheap `Clone` (one `Arc`), so a source impl holds
//! one and every call goes to the same process.
//!
//! Transport choice: the pipe *is* the capability. Nothing else on the machine
//! can address it, so there is no port to allocate and no nonce to check, and
//! process lifetime equals connection lifetime — crash handling is plain child
//! supervision. Audio bytes are the one exception and do not come back this
//! way: the player only ever consumes a URL, so a plugin serves its own bytes
//! and returns that URL from `resolve_stream`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::sync::{broadcast, mpsc, oneshot};

use super::manifest::PluginManifest;
use super::wire::{
    self, ErrorKind, Incoming, InitializeParams, InitializeResult, MAX_LINE_BYTES, Notification,
    PROTOCOL_VERSION, Request, RpcError,
};
use crate::source::{Capabilities, SourceError};

/// How long a plugin has to answer an ordinary call.
const CALL_TIMEOUT: Duration = Duration::from_secs(60);
/// The sign-in wizard is paced by a person at a browser, so a plugin holding
/// `auth_submit` open until an OAuth callback arrives is working, not wedged.
const AUTH_TIMEOUT: Duration = Duration::from_secs(300);
/// Gap between health pings.
const PING_INTERVAL: Duration = Duration::from_secs(20);
/// A ping this slow counts as a failure.
const PING_TIMEOUT: Duration = Duration::from_secs(10);
/// Consecutive ping failures before the child is presumed wedged.
const PING_FAILURES_BEFORE_RESTART: u32 = 3;
/// Longest wait between restart attempts.
const MAX_BACKOFF: Duration = Duration::from_secs(30);
/// Restarts allowed inside [`RESTART_WINDOW`] before the host gives up.
const MAX_RESTARTS: usize = 5;
const RESTART_WINDOW: Duration = Duration::from_secs(5 * 60);
/// How long a plugin gets to act on `shutdown` before it is killed. Short on
/// purpose: this runs on the app's quit path, where the alternative is a
/// visible hang. A plugin with real teardown to do should do it promptly.
const SHUTDOWN_GRACE: Duration = Duration::from_millis(300);

/// The in-flight request table: request id → whoever is awaiting the reply.
type PendingMap = HashMap<u64, oneshot::Sender<Result<serde_json::Value, RpcError>>>;

/// Something a plugin told the host about, out of band.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginEvent {
    /// The plugin signed in or out on its own.
    AuthChanged { authenticated: bool },
    /// The plugin's remote library changed; a sync is worth running.
    LibraryChanged,
    /// The child process is gone.
    Exited { status: String },
}

/// A live (or restartable) plugin process.
#[derive(Clone)]
pub struct PluginClient {
    inner: Arc<Inner>,
}

struct Inner {
    manifest: PluginManifest,
    /// The current child's write half plus its kill switch. `None` between a
    /// crash and the next successful respawn.
    conn: tokio::sync::Mutex<Option<Conn>>,
    pending: Arc<Mutex<PendingMap>>,
    next_id: AtomicU64,
    /// Refreshed on every handshake, so capabilities follow a restarted binary.
    handshake: RwLock<InitializeResult>,
    events: broadcast::Sender<PluginEvent>,
    /// When each restart happened, trimmed to [`RESTART_WINDOW`].
    restarts: Mutex<Vec<Instant>>,
    /// Set once the host has given up respawning this plugin.
    exhausted: AtomicBool,
    /// Set by [`PluginClient::shutdown`] so the monitor does not respawn.
    stopping: AtomicBool,
}

struct Conn {
    stdin_tx: mpsc::UnboundedSender<String>,
    kill_tx: Option<oneshot::Sender<()>>,
    /// Aborted when the connection is replaced, so a dead child leaves no pumps.
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for Conn {
    fn drop(&mut self) {
        if let Some(kill) = self.kill_tx.take() {
            let _ = kill.send(());
        }
        for task in self.tasks.drain(..) {
            task.abort();
        }
    }
}

impl PluginClient {
    /// Spawn the plugin and complete its handshake. A protocol mismatch is a
    /// hard failure: the child is killed rather than spoken to in a dialect
    /// neither side agreed on.
    pub async fn connect(manifest: PluginManifest) -> Result<Self, SourceError> {
        let (tx, _) = broadcast::channel(32);
        let inner = Arc::new(Inner {
            manifest,
            conn: tokio::sync::Mutex::new(None),
            pending: Arc::new(Mutex::new(HashMap::new())),
            next_id: AtomicU64::new(1),
            handshake: RwLock::new(placeholder_handshake()),
            events: tx,
            restarts: Mutex::new(Vec::new()),
            exhausted: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
        });

        {
            let mut guard = inner.conn.lock().await;
            Inner::start_locked(&inner, &mut guard).await?;
        }

        let client = Self { inner };
        client.spawn_health_loop();
        Ok(client)
    }

    /// Invoke one method and decode its result.
    pub async fn call<P: Serialize, R: DeserializeOwned>(
        &self,
        method: &str,
        params: P,
    ) -> Result<R, SourceError> {
        let value = self.call_raw(method, params).await?;
        serde_json::from_value(value).map_err(|e| {
            SourceError::Backend(format!(
                "plugin {} sent an undecodable {method} result: {e}",
                self.inner.manifest.id
            ))
        })
    }

    /// Invoke one method, leaving the result as raw JSON.
    /// Ordinary calls answer promptly or something is wrong. The auth wizard is
    /// the exception, since it waits on a person.
    fn deadline_for(method: &str) -> Duration {
        match method {
            wire::method::AUTH_BEGIN | wire::method::AUTH_SUBMIT => AUTH_TIMEOUT,
            _ => CALL_TIMEOUT,
        }
    }

    pub async fn call_raw<P: Serialize>(
        &self,
        method: &str,
        params: P,
    ) -> Result<serde_json::Value, SourceError> {
        let params = serde_json::to_value(params).map_err(|e| {
            SourceError::InvalidInput(format!("cannot encode {method} params: {e}"))
        })?;
        self.inner
            .request(method, params, Self::deadline_for(method))
            .await
    }

    /// Fire a notification. Best-effort by definition — there is no reply to
    /// wait for, and a dead child is the monitor's problem, not the caller's.
    pub fn notify(&self, method: &str, params: serde_json::Value) {
        let inner = self.inner.clone();
        let method = method.to_string();
        tokio::spawn(async move {
            inner.send_notification(&method, params).await;
        });
    }

    /// What the running binary said it can do, from the last handshake.
    pub fn capabilities(&self) -> Capabilities {
        self.read_handshake(|h| h.capabilities)
    }

    /// True when the plugin still needs its sign-in wizard run.
    pub fn auth_required(&self) -> bool {
        self.read_handshake(|h| h.auth_required)
    }

    /// The signed-in account label, when the plugin reported one.
    pub fn account(&self) -> Option<String> {
        self.read_handshake(|h| h.account.clone())
    }

    /// The plugin's self-reported name and version from the handshake.
    pub fn identity(&self) -> (String, String) {
        self.read_handshake(|h| (h.name.clone(), h.version.clone()))
    }

    /// A track's public web page, from the handshake's `{id}` template.
    pub fn web_url(&self, item_id: &str) -> Option<String> {
        self.read_handshake(|h| h.web_url_template.clone())
            .map(|t| t.replace("{id}", item_id))
    }

    pub fn manifest(&self) -> &PluginManifest {
        &self.inner.manifest
    }

    /// Out-of-band plugin events. Late subscribers miss earlier ones — this is
    /// a nudge channel, not a log.
    pub fn subscribe(&self) -> broadcast::Receiver<PluginEvent> {
        self.inner.events.subscribe()
    }

    /// True once the host has stopped respawning this plugin after too many
    /// crashes. Cleared by the next explicit reconnect.
    pub fn is_exhausted(&self) -> bool {
        self.inner.exhausted.load(Ordering::Relaxed)
    }

    /// Forget the crash history and allow respawning again — what a user
    /// switching back to the source means.
    pub fn reset_restart_budget(&self) {
        self.inner.exhausted.store(false, Ordering::Relaxed);
        if let Ok(mut restarts) = self.inner.restarts.lock() {
            restarts.clear();
        }
    }

    /// Ask the plugin to stop, then make sure it did.
    pub async fn shutdown(&self) {
        self.inner.stopping.store(true, Ordering::Relaxed);
        self.inner
            .send_notification(wire::method::SHUTDOWN, serde_json::json!({}))
            .await;

        // Dropping the connection closes stdin (ending the writer task) and
        // fires the kill switch, which races the child's own exit.
        tokio::time::sleep(SHUTDOWN_GRACE).await;
        let mut guard = self.inner.conn.lock().await;
        guard.take();
        self.inner.fail_pending(SourceError::Backend(format!(
            "plugin {} was shut down",
            self.inner.manifest.id
        )));
    }

    fn read_handshake<T>(&self, f: impl FnOnce(&InitializeResult) -> T) -> T {
        match self.inner.handshake.read() {
            Ok(guard) => f(&guard),
            // A poisoned lock means a panic while formatting the handshake;
            // the data is still structurally valid, so read it anyway rather
            // than losing the source entirely.
            Err(poisoned) => f(&poisoned.into_inner()),
        }
    }

    /// The health ping. Holds a `Weak` so dropping every client stops it.
    fn spawn_health_loop(&self) {
        let weak = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            let mut failures = 0u32;
            loop {
                tokio::time::sleep(PING_INTERVAL).await;
                let Some(inner) = Weak::upgrade(&weak) else {
                    return;
                };
                if inner.stopping.load(Ordering::Relaxed) {
                    return;
                }
                let result = inner
                    .request(wire::method::PING, serde_json::json!({}), PING_TIMEOUT)
                    .await;
                match result {
                    Ok(_) => failures = 0,
                    Err(e) => {
                        failures += 1;
                        tracing::warn!(
                            plugin = %inner.manifest.id,
                            failures,
                            error = %e,
                            "plugin health ping failed"
                        );
                        if failures >= PING_FAILURES_BEFORE_RESTART {
                            failures = 0;
                            inner.force_restart().await;
                        }
                    }
                }
            }
        });
    }
}

impl Inner {
    /// Send a request and await its reply, respawning first if the child died.
    async fn request(
        self: &Arc<Self>,
        method: &str,
        params: serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value, SourceError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        let line = serde_json::to_string(&Request::new(id, method, params)).map_err(|e| {
            SourceError::InvalidInput(format!("cannot encode {method} request: {e}"))
        })?;

        {
            let mut pending = self.lock_pending();
            pending.insert(id, tx);
        }

        if let Err(e) = self.send_line(line).await {
            self.lock_pending().remove(&id);
            return Err(e);
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Ok(value))) => Ok(value),
            Ok(Ok(Err(rpc))) => Err(rpc_to_source_error(&self.manifest.id, method, rpc)),
            // The sender was dropped without a value: the child died and the
            // exit path did not reach this entry. Never leave a caller hanging.
            Ok(Err(_)) => Err(SourceError::Backend(format!(
                "plugin {} dropped the {method} request",
                self.manifest.id
            ))),
            Err(_) => {
                self.lock_pending().remove(&id);
                Err(SourceError::Backend(format!(
                    "plugin {} did not answer {method} within {}s",
                    self.manifest.id,
                    timeout.as_secs()
                )))
            }
        }
    }

    async fn send_notification(self: &Arc<Self>, method: &str, params: serde_json::Value) {
        match serde_json::to_string(&Notification::new(method, params)) {
            Ok(line) => {
                if let Err(e) = self.send_line(line).await {
                    tracing::debug!(
                        plugin = %self.manifest.id,
                        method,
                        error = %e,
                        "dropping notification to a plugin that is not running"
                    );
                }
            }
            Err(e) => tracing::warn!(
                plugin = %self.manifest.id,
                method,
                error = %e,
                "cannot encode plugin notification"
            ),
        }
    }

    /// Write one framed line, standing the child back up if it is gone.
    async fn send_line(self: &Arc<Self>, line: String) -> Result<(), SourceError> {
        let mut guard = self.conn.lock().await;
        if guard.is_none() {
            Self::start_locked(self, &mut guard).await?;
        }
        let conn = guard.as_ref().ok_or_else(|| {
            SourceError::Backend(format!("plugin {} is not running", self.manifest.id))
        })?;
        conn.stdin_tx.send(line).map_err(|_| {
            SourceError::Backend(format!("plugin {} closed its input", self.manifest.id))
        })
    }

    /// Kill the current child so the next call respawns it.
    async fn force_restart(self: &Arc<Self>) {
        let mut guard = self.conn.lock().await;
        guard.take();
        drop(guard);
        self.fail_pending(SourceError::Backend(format!(
            "plugin {} stopped responding and was restarted",
            self.manifest.id
        )));
    }

    /// Spawn + handshake. The caller holds the connection lock, which is what
    /// serialises concurrent callers into a single respawn.
    async fn start_locked(self: &Arc<Self>, guard: &mut Option<Conn>) -> Result<(), SourceError> {
        if self.exhausted.load(Ordering::Relaxed) {
            return Err(SourceError::Backend(format!(
                "plugin {} crashed repeatedly and was stopped",
                self.manifest.id
            )));
        }

        if let Some(delay) = self.next_backoff() {
            tracing::info!(
                plugin = %self.manifest.id,
                delay_secs = delay.as_secs(),
                "waiting before restarting plugin"
            );
            tokio::time::sleep(delay).await;
        }

        let (conn, handshake) = self.spawn_and_handshake().await?;
        if let Ok(mut slot) = self.handshake.write() {
            *slot = handshake;
        }
        *guard = Some(conn);
        Ok(())
    }

    /// Record this attempt and report how long to wait first, or `None` for the
    /// very first start. Gives up entirely past [`MAX_RESTARTS`] in the window.
    fn next_backoff(&self) -> Option<Duration> {
        let Ok(mut restarts) = self.restarts.lock() else {
            return None;
        };
        let now = Instant::now();
        restarts.retain(|t| now.duration_since(*t) < RESTART_WINDOW);
        let attempt = restarts.len();
        restarts.push(now);

        if attempt > MAX_RESTARTS {
            self.exhausted.store(true, Ordering::Relaxed);
            tracing::error!(
                plugin = %self.manifest.id,
                "plugin restarted more than {MAX_RESTARTS} times in 5 minutes; giving up"
            );
            return None;
        }
        (attempt > 0).then(|| {
            let secs = 1u64 << (attempt as u32 - 1).min(4);
            Duration::from_secs(secs).min(MAX_BACKOFF)
        })
    }

    async fn spawn_and_handshake(
        self: &Arc<Self>,
    ) -> Result<(Conn, InitializeResult), SourceError> {
        let data_dir = self.manifest.data_dir();
        if let Err(e) = std::fs::create_dir_all(&data_dir) {
            return Err(SourceError::Backend(format!(
                "cannot create the data directory for plugin {}: {e}",
                self.manifest.id
            )));
        }

        let exe = self.manifest.executable_path();
        let mut command = Command::new(&exe);
        command
            .args(&self.manifest.args)
            .current_dir(&self.manifest.dir)
            .env("KOPUZ_PLUGIN_DATA_DIR", &data_dir)
            .env("KOPUZ_PLUGIN_PROTOCOL", PROTOCOL_VERSION.to_string())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        let mut child = command.spawn().map_err(|e| {
            SourceError::Backend(format!(
                "cannot start plugin {} ({}): {e}",
                self.manifest.id,
                exe.display()
            ))
        })?;

        let (stdin, stdout, stderr) =
            match (child.stdin.take(), child.stdout.take(), child.stderr.take()) {
                (Some(i), Some(o), Some(e)) => (i, o, e),
                _ => {
                    let _ = child.kill().await;
                    return Err(SourceError::Backend(format!(
                        "plugin {} did not expose its pipes",
                        self.manifest.id
                    )));
                }
            };

        let (stdin_tx, stdin_rx) = mpsc::unbounded_channel::<String>();
        let (kill_tx, kill_rx) = oneshot::channel::<()>();
        let id = self.manifest.id.clone();

        let tasks = vec![
            tokio::spawn(writer_task(id.clone(), stdin, stdin_rx)),
            tokio::spawn(reader_task(
                id.clone(),
                stdout,
                self.pending.clone(),
                self.events.clone(),
            )),
            tokio::spawn(stderr_task(id.clone(), stderr)),
            tokio::spawn(monitor_task(Arc::downgrade(self), child, kill_rx)),
        ];

        let conn = Conn {
            stdin_tx,
            kill_tx: Some(kill_tx),
            tasks,
        };

        match self.handshake_over(&conn).await {
            Ok(handshake) => {
                tracing::info!(
                    plugin = %id,
                    name = %handshake.name,
                    version = %handshake.version,
                    "plugin connected"
                );
                Ok((conn, handshake))
            }
            Err(e) => {
                // Dropping the connection fires the kill switch, so a plugin
                // that fails version negotiation never stays resident.
                drop(conn);
                Err(e)
            }
        }
    }

    /// The `initialize` exchange, sent directly over `conn` because the
    /// connection is not installed yet.
    async fn handshake_over(
        self: &Arc<Self>,
        conn: &Conn,
    ) -> Result<InitializeResult, SourceError> {
        let params = InitializeParams {
            protocol: PROTOCOL_VERSION,
            host_version: env!("CARGO_PKG_VERSION").to_string(),
            locale: i18n_locale(),
            data_dir: self.manifest.data_dir(),
        };
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let params = serde_json::to_value(params)
            .map_err(|e| SourceError::Backend(format!("cannot encode initialize: {e}")))?;
        let line = serde_json::to_string(&Request::new(id, wire::method::INITIALIZE, params))
            .map_err(|e| SourceError::Backend(format!("cannot encode initialize: {e}")))?;

        let (tx, rx) = oneshot::channel();
        self.lock_pending().insert(id, tx);
        conn.stdin_tx.send(line).map_err(|_| {
            SourceError::Backend(format!(
                "plugin {} closed its input during the handshake",
                self.manifest.id
            ))
        })?;

        let value = match tokio::time::timeout(CALL_TIMEOUT, rx).await {
            Ok(Ok(Ok(value))) => value,
            Ok(Ok(Err(rpc))) => {
                return Err(rpc_to_source_error(
                    &self.manifest.id,
                    wire::method::INITIALIZE,
                    rpc,
                ));
            }
            Ok(Err(_)) => {
                return Err(SourceError::Backend(format!(
                    "plugin {} exited during the handshake",
                    self.manifest.id
                )));
            }
            Err(_) => {
                self.lock_pending().remove(&id);
                return Err(SourceError::Backend(format!(
                    "plugin {} did not complete its handshake in time",
                    self.manifest.id
                )));
            }
        };

        let handshake: InitializeResult = serde_json::from_value(value).map_err(|e| {
            SourceError::Backend(format!(
                "plugin {} sent a malformed handshake: {e}",
                self.manifest.id
            ))
        })?;
        if handshake.protocol != PROTOCOL_VERSION {
            return Err(SourceError::Backend(format!(
                "plugin {} speaks protocol {}, this Kopuz speaks {PROTOCOL_VERSION}",
                handshake.name, handshake.protocol
            )));
        }
        Ok(handshake)
    }

    /// Resolve every in-flight request with `error`. Called on child exit so a
    /// crash surfaces as a failed operation rather than a hung UI.
    fn fail_pending(&self, error: SourceError) {
        let waiting: Vec<_> = self.lock_pending().drain().map(|(_, tx)| tx).collect();
        if waiting.is_empty() {
            return;
        }
        let rpc = source_error_to_rpc(&error);
        for tx in waiting {
            let _ = tx.send(Err(rpc.clone()));
        }
    }

    fn lock_pending(
        &self,
    ) -> std::sync::MutexGuard<'_, HashMap<u64, oneshot::Sender<Result<serde_json::Value, RpcError>>>>
    {
        // A panic while holding this map would only have left a half-inserted
        // entry; recovering keeps the plugin usable instead of poisoning the
        // whole source.
        self.pending.lock().unwrap_or_else(|e| e.into_inner())
    }
}

// ============================== stdio pumps ==============================

async fn writer_task(
    id: String,
    mut stdin: tokio::process::ChildStdin,
    mut rx: mpsc::UnboundedReceiver<String>,
) {
    while let Some(line) = rx.recv().await {
        if stdin.write_all(line.as_bytes()).await.is_err()
            || stdin.write_all(b"\n").await.is_err()
            || stdin.flush().await.is_err()
        {
            tracing::debug!(plugin = %id, "plugin input closed");
            return;
        }
    }
    // The channel closed: drop stdin so the plugin sees EOF and can exit.
    tracing::debug!(plugin = %id, "closing plugin input");
}

async fn reader_task(
    id: String,
    stdout: tokio::process::ChildStdout,
    pending: Arc<Mutex<PendingMap>>,
    events: broadcast::Sender<PluginEvent>,
) {
    let mut lines = LineReader::new(stdout);
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<Incoming>(&line) {
                    Ok(Incoming::Response(resp)) => {
                        let waiting = pending
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .remove(&resp.id);
                        match waiting {
                            Some(tx) => {
                                let outcome = match (resp.result, resp.error) {
                                    (_, Some(err)) => Err(err),
                                    (Some(value), None) => Ok(value),
                                    (None, None) => Ok(serde_json::Value::Null),
                                };
                                let _ = tx.send(outcome);
                            }
                            None => tracing::debug!(
                                plugin = %id,
                                id = resp.id,
                                "reply to an unknown or timed-out request"
                            ),
                        }
                    }
                    Ok(Incoming::Notification(notif)) => {
                        handle_notification(&id, notif, &events);
                    }
                    Err(e) => tracing::warn!(
                        plugin = %id,
                        error = %e,
                        "unparseable line from plugin"
                    ),
                }
            }
            Ok(None) => {
                tracing::debug!(plugin = %id, "plugin output closed");
                return;
            }
            Err(e) => {
                tracing::warn!(plugin = %id, error = %e, "plugin output failed");
                return;
            }
        }
    }
}

fn handle_notification(id: &str, notif: Notification, events: &broadcast::Sender<PluginEvent>) {
    match notif.method.as_str() {
        wire::event::LOG => match serde_json::from_value::<wire::LogParams>(notif.params) {
            Ok(log) => emit_plugin_log(id, &log),
            Err(e) => tracing::warn!(plugin = %id, error = %e, "malformed plugin log"),
        },
        wire::event::AUTH_CHANGED => {
            match serde_json::from_value::<wire::AuthChangedParams>(notif.params) {
                Ok(p) => {
                    let _ = events.send(PluginEvent::AuthChanged {
                        authenticated: p.authenticated,
                    });
                }
                Err(e) => tracing::warn!(plugin = %id, error = %e, "malformed auth_changed"),
            }
        }
        wire::event::LIBRARY_CHANGED => {
            let _ = events.send(PluginEvent::LibraryChanged);
        }
        other => tracing::debug!(plugin = %id, method = other, "unknown plugin notification"),
    }
}

fn emit_plugin_log(id: &str, log: &wire::LogParams) {
    let target = log.target.as_deref().unwrap_or("");
    match log.level {
        wire::LogLevel::Trace => {
            tracing::trace!(target: "plugin", plugin = %id, module = target, "{}", log.message);
        }
        wire::LogLevel::Debug => {
            tracing::debug!(target: "plugin", plugin = %id, module = target, "{}", log.message);
        }
        wire::LogLevel::Info => {
            tracing::info!(target: "plugin", plugin = %id, module = target, "{}", log.message);
        }
        wire::LogLevel::Warn => {
            tracing::warn!(target: "plugin", plugin = %id, module = target, "{}", log.message);
        }
        wire::LogLevel::Error => {
            tracing::error!(target: "plugin", plugin = %id, module = target, "{}", log.message);
        }
    }
}

async fn stderr_task(id: String, stderr: tokio::process::ChildStderr) {
    let mut lines = LineReader::new(stderr);
    while let Ok(Some(line)) = lines.next_line().await {
        if !line.trim().is_empty() {
            tracing::debug!(target: "plugin", plugin = %id, "{}", line.trim_end());
        }
    }
}

/// Waits for the child, then makes its death visible: every in-flight request
/// fails and the connection slot empties so the next call respawns.
async fn monitor_task(inner: Weak<Inner>, mut child: Child, kill_rx: oneshot::Receiver<()>) {
    let status = tokio::select! {
        status = child.wait() => match status {
            Ok(status) => status.to_string(),
            Err(e) => format!("wait failed: {e}"),
        },
        _ = kill_rx => {
            let _ = child.kill().await;
            "killed by the host".to_string()
        }
    };

    let Some(inner) = Weak::upgrade(&inner) else {
        return;
    };
    if inner.stopping.load(Ordering::Relaxed) {
        return;
    }

    tracing::warn!(plugin = %inner.manifest.id, status, "plugin exited");
    inner.fail_pending(SourceError::Backend(format!(
        "plugin {} exited: {status}",
        inner.manifest.id
    )));
    let _ = inner.events.send(PluginEvent::Exited { status });

    // Clearing the slot is what lets the next call respawn. `try_lock` because
    // this may run while a caller holds the lock mid-respawn — in which case a
    // fresh child is already on its way and clearing it would be wrong.
    if let Ok(mut guard) = inner.conn.try_lock() {
        guard.take();
    }
}

// =========================== framing primitives ==========================

/// Newline framing with a hard cap. `BufReader::lines` would let a child drive
/// an unbounded allocation, which a plugin host must not allow.
struct LineReader<R> {
    inner: R,
    buf: Vec<u8>,
    scanned: usize,
}

impl<R: AsyncRead + Unpin> LineReader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            buf: Vec::with_capacity(8 * 1024),
            scanned: 0,
        }
    }

    async fn next_line(&mut self) -> std::io::Result<Option<String>> {
        let mut chunk = [0u8; 8 * 1024];
        loop {
            if let Some(offset) = self.buf[self.scanned..].iter().position(|b| *b == b'\n') {
                let end = self.scanned + offset;
                let line = String::from_utf8_lossy(&self.buf[..end]).into_owned();
                self.buf.drain(..=end);
                self.scanned = 0;
                return Ok(Some(line));
            }
            self.scanned = self.buf.len();

            let read = self.inner.read(&mut chunk).await?;
            if read == 0 {
                if self.buf.is_empty() {
                    return Ok(None);
                }
                let line = String::from_utf8_lossy(&self.buf).into_owned();
                self.buf.clear();
                self.scanned = 0;
                return Ok(Some(line));
            }
            self.buf.extend_from_slice(&chunk[..read]);
            if self.buf.len() > MAX_LINE_BYTES {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("line exceeded {MAX_LINE_BYTES} bytes"),
                ));
            }
        }
    }
}

// ============================ error translation ==========================

fn rpc_to_source_error(plugin_id: &str, method: &str, err: RpcError) -> SourceError {
    let kind = err.data.map(|d| d.kind).unwrap_or(ErrorKind::Backend);
    let message = if err.message.trim().is_empty() {
        format!("plugin {plugin_id} failed {method}")
    } else {
        err.message
    };
    match kind {
        ErrorKind::Unsupported => SourceError::unsupported_owned(message),
        ErrorKind::Connectivity => SourceError::Connectivity,
        ErrorKind::Auth => SourceError::Auth,
        ErrorKind::InvalidInput => SourceError::InvalidInput(message),
        ErrorKind::Backend => SourceError::Backend(message),
    }
}

fn source_error_to_rpc(error: &SourceError) -> RpcError {
    let kind = match error {
        SourceError::Unsupported(_) => ErrorKind::Unsupported,
        SourceError::Connectivity => ErrorKind::Connectivity,
        SourceError::Auth => ErrorKind::Auth,
        SourceError::InvalidInput(_) => ErrorKind::InvalidInput,
        SourceError::Backend(_) => ErrorKind::Backend,
    };
    RpcError {
        code: -32000,
        message: error.to_string(),
        data: Some(wire::ErrorData { kind }),
    }
}

/// What to report before a handshake has landed. Never observed by a caller —
/// [`PluginClient::connect`] only returns after a real handshake replaces it.
fn placeholder_handshake() -> InitializeResult {
    InitializeResult {
        protocol: PROTOCOL_VERSION,
        name: String::new(),
        version: String::new(),
        capabilities: Capabilities::default(),
        auth_required: true,
        data_base_url: String::new(),
        data_token: String::new(),
        account: None,
        web_url_template: None,
    }
}

fn i18n_locale() -> String {
    std::env::var("LANG")
        .ok()
        .and_then(|l| l.split('.').next().map(str::to_owned))
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| "en".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn line_reader_frames_and_caps() {
        let payload = b"one\ntwo\nthree".to_vec();
        let mut reader = LineReader::new(std::io::Cursor::new(payload));
        assert_eq!(reader.next_line().await.expect("read"), Some("one".into()));
        assert_eq!(reader.next_line().await.expect("read"), Some("two".into()));
        assert_eq!(
            reader.next_line().await.expect("read"),
            Some("three".into())
        );
        assert_eq!(reader.next_line().await.expect("read"), None);
    }

    #[tokio::test]
    async fn line_reader_rejects_an_overlong_line() {
        let payload = vec![b'x'; MAX_LINE_BYTES + 16];
        let mut reader = LineReader::new(std::io::Cursor::new(payload));
        assert!(reader.next_line().await.is_err());
    }

    #[test]
    fn rpc_errors_map_onto_source_errors() {
        let mk = |kind: ErrorKind| RpcError {
            code: -32000,
            message: "boom".into(),
            data: Some(wire::ErrorData { kind }),
        };
        assert_eq!(
            rpc_to_source_error("p", "m", mk(ErrorKind::Auth)),
            SourceError::Auth
        );
        assert_eq!(
            rpc_to_source_error("p", "m", mk(ErrorKind::Unsupported)),
            SourceError::unsupported_owned("boom".to_string())
        );
        assert_eq!(
            rpc_to_source_error("p", "m", mk(ErrorKind::Connectivity)),
            SourceError::Connectivity
        );
        // No `data` at all still classifies rather than panicking.
        assert_eq!(
            rpc_to_source_error(
                "p",
                "m",
                RpcError {
                    code: -1,
                    message: String::new(),
                    data: None
                }
            ),
            SourceError::Backend("plugin p failed m".to_string())
        );
    }
}
