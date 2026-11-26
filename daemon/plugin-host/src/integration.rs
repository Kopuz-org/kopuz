//! An integration plugin, opened from the person's answers to its fields. It
//! keeps one instance, so a connection the plugin holds stays open between
//! calls, and runs its calls one at a time in the order they were made. The
//! plugin under it can be swapped while it runs, like a source session's.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use tokio::sync::Mutex;
use wasmtime::component::ResourceAny;

use crate::integration_bindings::IntegrationPlugin;
use crate::plugin::{Instance, Plugin};
use crate::{PluginError, wit};

/// An integration answers what kopuz is doing as it happens, so a call still
/// going after this is stuck, not busy. Past the host's own request timeout,
/// so a slow request fails as itself before the whole call is given up on.
const CALL_LIMIT: Duration = if cfg!(test) {
    Duration::from_secs(1)
} else {
    crate::host::DEFAULT_TIMEOUT.saturating_add(Duration::from_secs(10))
};

struct Live {
    plugin: Arc<Plugin>,
    instance: Instance<IntegrationPlugin>,
    handle: ResourceAny,
}

pub struct Integration {
    plugin: RwLock<Arc<Plugin>>,
    settings: Vec<(String, String)>,
    live: Mutex<Option<Live>>,
    closed: AtomicBool,
}

impl std::fmt::Debug for Integration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Integration")
            .field("plugin", &self.plugin().id())
            .finish_non_exhaustive()
    }
}

impl Integration {
    /// Nothing runs until the first call.
    pub fn new(plugin: Arc<Plugin>, settings: Vec<(String, String)>) -> Self {
        Self {
            plugin: RwLock::new(plugin),
            settings,
            live: Mutex::new(None),
            closed: AtomicBool::new(false),
        }
    }

    pub fn plugin(&self) -> Arc<Plugin> {
        Arc::clone(
            &self
                .plugin
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }

    pub fn settings(&self) -> &[(String, String)] {
        &self.settings
    }

    /// Run every later call on `plugin`; the instance of the old code is
    /// closed before the next one.
    pub fn swap(&self, plugin: Arc<Plugin>) {
        *self
            .plugin
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = plugin;
        // An idle instance of the old code goes now, with whatever connection it
        // holds; one busy in a call is replaced before the next.
        if let Ok(mut held) = self.live.try_lock() {
            held.take();
        }
    }

    /// Drop the running instance and whatever connections it held, for good:
    /// a call still queued for it fails instead of opening another, so a
    /// device that takes one client at a time is free for whatever replaces it.
    pub async fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.live.lock().await.take();
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    async fn open(&self, plugin: Arc<Plugin>) -> Result<Live, PluginError> {
        let server_url = plugin
            .manifest()
            .integration
            .as_ref()
            .map(|integration| integration.server_url(&self.settings).to_string())
            .unwrap_or_default();
        let mut instance = plugin.instantiate_integration(&server_url).await?;
        let settings: Vec<wit::Setting> = self
            .settings
            .iter()
            .map(|(key, value)| wit::Setting {
                key: key.clone(),
                value: value.clone(),
            })
            .collect();
        let constructed = plugin
            .starting(
                "opening an integration",
                instance
                    .bindings
                    .kopuz_plugin_integration()
                    .instance()
                    .call_constructor(&mut instance.store, &settings),
            )
            .await?;
        let handle = constructed.map_err(|error| plugin.trapped(error))?;
        Ok(Live {
            plugin,
            instance,
            handle,
        })
    }
}

/// One method per instance call. The instance is taken out of `live` for the
/// call and put back only once it returns: one that trapped, timed out or was
/// dropped mid-call goes with it, since its memory may be half-written and a
/// component cannot be entered again after a call it never finished.
macro_rules! calls {
    ($(
        $(#[$meta:meta])*
        fn $name:ident => $call:ident ( $( $arg:ident : $ty:ty => $pass:expr ),* ) -> $ret:ty;
    )*) => {
        impl Integration {
            $(
                $(#[$meta])*
                pub async fn $name(&self, $( $arg: $ty ),*) -> Result<$ret, PluginError> {
                    let mut held = self.live.lock().await;
                    if self.is_closed() {
                        return Err(PluginError::Runtime("the integration was closed".into()));
                    }
                    let current = self.plugin();
                    let mut live = match held.take() {
                        Some(live) if Arc::ptr_eq(&live.plugin, &current) => live,
                        _ => self.open(current).await?,
                    };
                    // Closed while it opened: what it opened goes unused.
                    if self.is_closed() {
                        return Err(PluginError::Runtime("the integration was closed".into()));
                    }
                    let guest = live.instance.bindings.kopuz_plugin_integration().instance();
                    let call = guest.$call(&mut live.instance.store, live.handle, $( $pass ),*);
                    match tokio::time::timeout(CALL_LIMIT, call).await {
                        Ok(Ok(value)) => {
                            *held = Some(live);
                            Ok(value)
                        }
                        Ok(Err(error)) => Err(live.plugin.trapped(error)),
                        Err(_) => {
                            tracing::error!(plugin = %live.plugin.id(), "integration call timed out");
                            Err(PluginError::TimedOut)
                        }
                    }
                }
            )*
        }
    };
}

type Fallible<T> = Result<T, wit::SourceError>;

calls! {
    /// Whether the settings work.
    fn validate => call_validate() -> Fallible<()>;

    fn on_playback => call_on_playback(event: &wit::PlaybackEvent => event) -> ();

    fn on_audio => call_on_audio(frames: &[wit::AudioFrame] => frames) -> ();
}
