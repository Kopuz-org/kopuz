//! The kopuzd side: a tokio runtime on its own thread, a `GrpcApi` over TCP,
//! and the three calls the screen needs. Results go back to the GPUI thread
//! as [`Msg`] over an unbounded futures channel; nothing here blocks the UI.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use api::{
    KopuzApi, Page, Phase, PlayerCommand, QueueContext, QueueMode, SetQueueRequest, TrackFilter,
};
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use tokio::runtime::Handle;

/// Where kopuzd listens (`kopuzd --listen`), reached through `adb reverse`.
pub const DAEMON_ADDR: &str = match option_env!("KOPUZ_SPIKE_ADDR") {
    Some(addr) => addr,
    None => "127.0.0.1:50777",
};

const PAGE_LIMIT: u32 = 500;
const CONNECT_TRIES: u32 = 20;

/// One list row.
#[derive(Debug, Clone)]
pub struct Row {
    pub key: String,
    pub title: String,
    pub artist: String,
}

/// What the runtime reports back to the UI.
#[derive(Debug)]
pub enum Msg {
    Connecting(u32),
    Connected,
    Tracks(Vec<Row>),
    Playing(bool),
    Error(String),
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

fn report(tx: &UnboundedSender<Msg>, text: String) {
    log::info!("SPIKE error {text}");
    let _ = tx.unbounded_send(Msg::Error(text));
}

fn is_playing(phase: Phase) -> bool {
    phase == Phase::Playing
}

pub struct Daemon {
    handle: Handle,
    api: Option<Arc<dyn KopuzApi>>,
    tx: UnboundedSender<Msg>,
}

impl Daemon {
    /// Build the (lazy) client. Nothing is dialled until the first call.
    pub fn new(handle: Handle, addr: &str) -> (Self, UnboundedReceiver<Msg>) {
        let (tx, rx) = unbounded();
        // tonic's lazy channel spawns its worker, so build it inside the runtime.
        let api = {
            let _guard = handle.enter();
            client::GrpcApi::connect_tcp(addr)
        };
        let api = match api {
            Ok(api) => Some(Arc::new(api) as Arc<dyn KopuzApi>),
            Err(error) => {
                report(&tx, format!("{addr}: {error}"));
                None
            }
        };
        (Self { handle, api, tx }, rx)
    }

    fn spawn<F, Fut>(&self, f: F)
    where
        F: FnOnce(Arc<dyn KopuzApi>, UnboundedSender<Msg>) -> Fut,
        Fut: Future<Output = ()> + Send + 'static,
    {
        match &self.api {
            Some(api) => {
                self.handle.spawn(f(Arc::clone(api), self.tx.clone()));
            }
            None => report(&self.tx, "no daemon client".into()),
        }
    }

    /// Probe the daemon (retrying while it is not up yet), then load the
    /// first page of tracks once.
    pub fn load(&self) {
        self.spawn(|api, tx| async move {
            let mut attempt = 0;
            let state = loop {
                attempt += 1;
                let _ = tx.unbounded_send(Msg::Connecting(attempt));
                let error = match tokio::time::timeout(Duration::from_secs(5), api.player_state())
                    .await
                {
                    Ok(Ok(state)) => break state,
                    Ok(Err(error)) => error.to_string(),
                    Err(_) => "timed out".to_string(),
                };
                report(&tx, format!("connect {DAEMON_ADDR} try {attempt}: {error}"));
                if attempt >= CONNECT_TRIES {
                    return;
                }
                tokio::time::sleep(Duration::from_secs(3)).await;
            };
            log::info!("SPIKE connected addr={DAEMON_ADDR}");
            let _ = tx.unbounded_send(Msg::Connected);
            let _ = tx.unbounded_send(Msg::Playing(is_playing(state.phase)));

            let page = Page {
                offset: 0,
                limit: PAGE_LIMIT,
            };
            match api.tracks(TrackFilter::default(), page).await {
                Ok(page) => {
                    let rows: Vec<Row> = page
                        .items
                        .into_iter()
                        .map(|t| Row {
                            key: t.key,
                            title: t.title,
                            artist: t.artist,
                        })
                        .collect();
                    log::info!("SPIKE tracks loaded n={}", rows.len());
                    let _ = tx.unbounded_send(Msg::Tracks(rows));
                }
                Err(error) => report(&tx, format!("tracks: {error}")),
            }
        });
    }

    /// Replace the queue with `keys` and start at `index`.
    pub fn play(&self, keys: Vec<String>, index: usize) {
        self.spawn(move |api, tx| async move {
            let request = SetQueueRequest {
                mode: QueueMode::Replace,
                context: QueueContext::Tracks { keys },
                start_index: Some(index as u32),
                shuffle: None,
            };
            match api.set_queue(request).await {
                Ok(_) => {
                    log::info!("SPIKE play ok index={index}");
                    let _ = tx.unbounded_send(Msg::Playing(true));
                }
                Err(error) => report(&tx, format!("play {index}: {error}")),
            }
        });
    }

    /// Play/pause, then re-read the phase so the button shows the truth.
    pub fn toggle(&self) {
        self.spawn(|api, tx| async move {
            match api.player_command(PlayerCommand::Toggle).await {
                Ok(_) => {
                    log::info!("SPIKE toggle ok");
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    if let Ok(state) = api.player_state().await {
                        let _ = tx.unbounded_send(Msg::Playing(is_playing(state.phase)));
                    }
                }
                Err(error) => report(&tx, format!("toggle: {error}")),
            }
        });
    }
}
