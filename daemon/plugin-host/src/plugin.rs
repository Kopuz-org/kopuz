//! One loaded plugin: its manifest, its code, and fresh instances of it.

use std::borrow::Cow;
use std::sync::{Arc, OnceLock};

use tokio::sync::OnceCell;
use wasmtime::component::{Component, HasSelf, Linker};
use wasmtime::{Config, Engine, Store};

use crate::bindings::SourcePlugin;
use crate::host::State;
use crate::manifest::{Manifest, Permissions};
use crate::{PluginError, wit};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Shipped inside kopuz.
    Bundled,
    /// Installed into the plugin directory.
    Installed,
}

/// How a plugin runs.
#[derive(Debug, Clone)]
pub enum Code {
    /// A component in the sandbox.
    Wasm(Cow<'static, [u8]>),
    /// Compiled into kopuz and run in-process, for a backend that needs what
    /// the sandbox does not give: a JS engine, DRM, a browser, sockets or the
    /// filesystem. Only kopuz ships these; the host supplies their code.
    Native,
}

pub struct Plugin {
    manifest: Manifest,
    origin: Origin,
    id: Arc<str>,
    permissions: Arc<Permissions>,
    code: Code,
    // A bundled one is compiled on first use, so services nobody configured
    // cost nothing; an installed one is compiled at load.
    component: OnceCell<Component>,
    digest: OnceLock<Option<String>>,
}

impl std::fmt::Debug for Plugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Plugin")
            .field("id", &self.id)
            .field("version", &self.manifest.version)
            .field("origin", &self.origin)
            .field("native", &self.is_native())
            .finish_non_exhaustive()
    }
}

pub(crate) struct Instance {
    pub(crate) store: Store<State>,
    pub(crate) bindings: SourcePlugin,
}

struct Runtime {
    engine: Engine,
    linker: Linker<State>,
}

fn runtime() -> Result<&'static Runtime, PluginError> {
    static RUNTIME: OnceLock<Result<Runtime, String>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            let build = || -> wasmtime::Result<Runtime> {
                let mut config = Config::new();
                config.epoch_interruption(true);
                let engine = Engine::new(&config)?;
                tick(&engine);
                let mut linker = Linker::new(&engine);
                wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
                SourcePlugin::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)?;
                Ok(Runtime { engine, linker })
            };
            build().map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|error| PluginError::Runtime(error.clone()))
}

fn compile(engine: &Engine, wasm: &[u8]) -> Result<Component, PluginError> {
    Component::new(engine, wasm).map_err(|error| PluginError::Invalid(format!("{error:#}")))
}

const EPOCH_TICK: std::time::Duration = std::time::Duration::from_millis(10);

/// Instantiating and constructing a session do no real work, so a plugin
/// still busy after this is stuck.
const STARTUP_LIMIT: std::time::Duration = if cfg!(test) {
    std::time::Duration::from_secs(1)
} else {
    std::time::Duration::from_secs(60)
};

/// Plugin code yields every tick, so one spinning in a loop costs CPU, not a
/// runtime worker.
fn tick(engine: &Engine) {
    let engine = engine.clone();
    let spawned = std::thread::Builder::new()
        .name("plugin-epoch".into())
        .spawn(move || {
            loop {
                std::thread::sleep(EPOCH_TICK);
                engine.increment_epoch();
            }
        });
    if let Err(error) = spawned {
        tracing::error!(%error, "the plugin epoch thread did not start; plugins will not yield");
    }
}

impl Plugin {
    pub fn new(manifest: Manifest, code: Code, origin: Origin) -> Self {
        Self {
            id: Arc::from(manifest.id.as_str()),
            permissions: Arc::new(manifest.permissions.clone()),
            manifest,
            origin,
            code,
            component: OnceCell::new(),
            digest: OnceLock::new(),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    pub fn origin(&self) -> Origin {
        self.origin
    }

    pub fn is_native(&self) -> bool {
        matches!(self.code, Code::Native)
    }

    /// The SHA-256 of the component in lowercase hex, as a marketplace lists
    /// it; none for a native plugin.
    pub fn digest(&self) -> Option<&str> {
        self.digest
            .get_or_init(|| match &self.code {
                Code::Wasm(wasm) => Some(crate::market::sha256_hex(wasm)),
                Code::Native => None,
            })
            .as_deref()
    }

    async fn component(&self) -> Result<&Component, PluginError> {
        let Code::Wasm(wasm) = &self.code else {
            return Err(PluginError::Invalid(format!(
                "{} runs in-process and has no component",
                self.id
            )));
        };
        self.component
            .get_or_try_init(|| async {
                let engine = runtime()?.engine.clone();
                let wasm = wasm.clone().into_owned();
                tokio::task::spawn_blocking(move || compile(&engine, &wasm))
                    .await
                    .map_err(|error| PluginError::Runtime(error.to_string()))?
            })
            .await
    }

    /// Compile the plugin on this thread, so one that does not compile is
    /// known before it is offered.
    pub fn compile(&self) -> Result<(), PluginError> {
        let Code::Wasm(wasm) = &self.code else {
            return Ok(());
        };
        if self.component.initialized() {
            return Ok(());
        }
        let component = compile(&runtime()?.engine, wasm)?;
        // A first call racing this one compiled the same code; keep either.
        let _ = self.component.set(component);
        Ok(())
    }

    /// Compile the plugin now, so the first call to it does not wait.
    pub async fn prepare(&self) -> Result<(), PluginError> {
        self.component().await.map(|_| ())
    }

    /// A fresh instance whose network is scoped to `server_url`.
    pub(crate) async fn instantiate(&self, server_url: &str) -> Result<Instance, PluginError> {
        let component = self.component().await?;
        let runtime = runtime()?;
        let state = State::new(
            Arc::clone(&self.id),
            Arc::clone(&self.permissions),
            server_url,
        );
        let mut store = Store::new(&runtime.engine, state);
        store.limiter(|state| &mut state.limits);
        store.set_epoch_deadline(1);
        store.epoch_deadline_async_yield_and_update(1);
        let started = self
            .starting(
                "instantiating",
                SourcePlugin::instantiate_async(&mut store, component, &runtime.linker),
            )
            .await?;
        let bindings = started.map_err(|error| self.trapped(error))?;
        Ok(Instance { store, bindings })
    }

    pub async fn login(
        &self,
        connection: wit::Connection,
        username: &str,
        password: &str,
    ) -> Result<wit::AuthSession, wit::SourceError> {
        let mut instance = self
            .instantiate(&connection.url)
            .await
            .map_err(PluginError::into_source_error)?;
        let login = instance.bindings.kopuz_plugin_source().call_login(
            &mut instance.store,
            &connection,
            username,
            password,
        );
        match tokio::time::timeout(crate::session::CALL_LIMIT, login).await {
            Ok(result) => result.map_err(|error| self.trapped(error).into_source_error())?,
            Err(_) => Err(PluginError::TimedOut.into_source_error()),
        }
    }

    /// `work`, unless it is still going after [`STARTUP_LIMIT`].
    pub(crate) async fn starting<T>(
        &self,
        doing: &str,
        work: impl std::future::Future<Output = T>,
    ) -> Result<T, PluginError> {
        tokio::time::timeout(STARTUP_LIMIT, work)
            .await
            .map_err(|_| {
                tracing::error!(plugin = %self.id, "plugin timed out {doing}");
                PluginError::TimedOut
            })
    }

    pub(crate) fn trapped(&self, error: wasmtime::Error) -> PluginError {
        tracing::error!(plugin = %self.id, error = %format!("{error:#}"), "plugin trapped");
        PluginError::Trapped(format!("{error:#}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn startup_that_never_finishes_times_out() {
        let manifest = Manifest::parse(
            "id = \"stuck\"\nname = \"Stuck\"\nversion = \"0.1.0\"\napi = \"0.1\"\n",
        )
        .expect("manifest");
        let plugin = Plugin::new(manifest, Code::Native, Origin::Installed);
        let started = plugin
            .starting("opening a session", std::future::pending::<()>())
            .await;
        assert!(matches!(started, Err(PluginError::TimedOut)));
    }
}
