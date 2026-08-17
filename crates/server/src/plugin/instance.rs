//! One loaded plugin: its sandbox, the table its entry chunk returned, and the
//! handshake `setup` answered with.
//!
//! Everything the rest of the app does to a plugin goes through [`call`] and
//! [`call_unit`], so this is the only place that knows a plugin operation is a
//! Lua function call with a deadline around it.
//!
//! [`call`]: PluginInstance::call
//! [`call_unit`]: PluginInstance::call_unit

use std::collections::HashSet;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use mlua::LuaSerdeExt;

use super::dto::export;
use super::{Handshake, PluginError, PluginManifest, api, lua};

/// Stands in for a method name when the entry chunk itself fails, so the error
/// reads the same way a failed call does.
const ENTRY: &str = "the entry chunk";

pub struct PluginInstance {
    manifest: PluginManifest,
    handshake: Handshake,
    /// Shared with the closures behind the `kopuz` global; `auth_ok` is how the
    /// plugin reports a sign-in change back to the host.
    host: Arc<api::HostCtx>,
    sandbox: lua::Sandbox,
    /// The table the entry chunk returned.
    table: mlua::Table,
    /// Its function-valued keys, snapshotted at load so declining an optional
    /// operation costs nothing.
    exports: HashSet<String>,
    /// One call at a time. mlua's own reentrant lock would serialize Lua
    /// execution anyway, and a single call in flight is what makes one shared
    /// deadline unambiguous.
    call_lock: tokio::sync::Mutex<()>,
}

impl PluginInstance {
    /// Build the sandbox, load the entry chunk, and run `setup(ctx)`.
    ///
    /// Both steps run under [`CALL_TIMEOUT`](super::CALL_TIMEOUT) and the deadline
    /// hook: a plugin that spins at load time fails to load instead of wedging
    /// whichever task asked for it.
    pub async fn load(manifest: PluginManifest) -> Result<Arc<Self>, PluginError> {
        let data_dir = manifest.data_dir();
        std::fs::create_dir_all(&data_dir)
            .map_err(|e| PluginError::Load(format!("cannot create {}: {e}", data_dir.display())))?;

        let host = Arc::new(api::HostCtx {
            plugin_id: manifest.id.clone(),
            plugin_name: manifest.name.clone(),
            data_dir: data_dir.clone(),
            locale: locale(),
            auth_ok: AtomicBool::new(false),
        });
        let sandbox = lua::build(&manifest, Arc::clone(&host))?;

        let entry = manifest.entry_path();
        let body = std::fs::read_to_string(&entry)
            .map_err(|e| PluginError::Load(format!("cannot read {}: {e}", entry.display())))?;
        let chunk = sandbox
            .lua
            .load(body)
            .set_name(format!("@{}", manifest.entry.display()));
        let returned = timed(
            &sandbox,
            ENTRY,
            super::CALL_TIMEOUT,
            chunk.eval_async::<mlua::Value>(),
        )
        .await
        .map_err(load_failed)?;

        let table = match returned {
            mlua::Value::Table(table) => table,
            other => {
                return Err(PluginError::Load(format!(
                    "{} must return a table, got {}",
                    manifest.entry.display(),
                    other.type_name()
                )));
            }
        };
        let exports = function_names(&table);

        let handshake = if exports.contains(export::SETUP) {
            let ctx =
                setup_ctx(&sandbox.lua, &manifest, &data_dir, &host.locale).map_err(load_failed)?;
            let setup = table
                .get::<mlua::Function>(export::SETUP)
                .map_err(load_failed)?;
            let returned = timed(
                &sandbox,
                export::SETUP,
                super::CALL_TIMEOUT,
                setup.call_async::<mlua::Value>(ctx),
            )
            .await
            .map_err(load_failed)?;
            deserialize(&sandbox.lua, export::SETUP, returned).map_err(load_failed)?
        } else {
            Handshake::default()
        };

        // A plugin with nothing to sign into is signed in by definition. One that
        // does starts out signed out, unless its `setup` already said otherwise.
        if !handshake.auth_required {
            host.auth_ok.store(true, Ordering::Relaxed);
        }

        tracing::debug!(
            plugin = %manifest.id,
            name = %handshake.name.as_deref().unwrap_or(&manifest.name),
            version = %handshake.version.as_deref().unwrap_or(&manifest.version),
            "plugin loaded"
        );

        Ok(Arc::new(Self {
            manifest,
            handshake,
            host,
            sandbox,
            table,
            exports,
            call_lock: tokio::sync::Mutex::new(()),
        }))
    }

    pub fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    pub fn handshake(&self) -> &Handshake {
        &self.handshake
    }

    /// Display name: the handshake's, falling back to the manifest's.
    pub fn name(&self) -> &str {
        self.handshake
            .name
            .as_deref()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(&self.manifest.name)
    }

    /// Whether the plugin's table has this function.
    pub fn exports(&self, method: &str) -> bool {
        self.exports.contains(method)
    }

    /// Whether the plugin last reported itself signed in.
    pub fn authenticated(&self) -> bool {
        self.host.auth_ok.load(Ordering::Relaxed)
    }

    /// Call an exported function and read its result through mlua's serde bridge.
    /// [`PluginError::Unsupported`] when the plugin does not export it, without
    /// entering Lua.
    pub async fn call<A, R>(&self, method: &'static str, args: A) -> Result<R, PluginError>
    where
        A: mlua::IntoLuaMulti + Send + 'static,
        R: serde::de::DeserializeOwned + Send + 'static,
    {
        let returned = self.invoke(method, args).await?;
        deserialize(&self.sandbox.lua, method, returned)
    }

    /// [`call`](Self::call) for a function whose return value is ignored.
    pub async fn call_unit<A>(&self, method: &'static str, args: A) -> Result<(), PluginError>
    where
        A: mlua::IntoLuaMulti + Send + 'static,
    {
        self.invoke(method, args).await.map(|_| ())
    }

    /// Run the plugin's optional `unload()`, best effort. Nothing is retried: the
    /// state is about to be dropped either way.
    pub async fn unload(&self) {
        if !self.exports(export::UNLOAD) {
            return;
        }
        if let Err(e) = self.call_unit(export::UNLOAD, ()).await {
            tracing::warn!(plugin = %self.manifest.id, error = %e, "plugin unload failed");
        }
    }

    async fn invoke<A>(&self, method: &'static str, args: A) -> Result<mlua::Value, PluginError>
    where
        A: mlua::IntoLuaMulti + Send + 'static,
    {
        if !self.exports(method) {
            return Err(PluginError::Unsupported(method.to_string()));
        }
        let _call = self.call_lock.lock().await;
        let function = self
            .table
            .get::<mlua::Function>(method)
            .map_err(|e| PluginError::from_lua(method, &e))?;
        timed(
            &self.sandbox,
            method,
            budget(method),
            function.call_async::<mlua::Value>(args),
        )
        .await
    }
}

/// Run one call against both halves of its deadline. The instruction hook stops a
/// spin loop inside Lua; the async timeout stops a host call that never comes
/// back, and plugin code that swallowed the hook's error with `pcall`.
async fn timed<T>(
    sandbox: &lua::Sandbox,
    method: &str,
    timeout: Duration,
    call: impl Future<Output = mlua::Result<T>>,
) -> Result<T, PluginError> {
    let _deadline = sandbox.arm(timeout);
    let elapsed = || PluginError::Timeout {
        method: method.to_string(),
    };
    match tokio::time::timeout(timeout, call).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) if lua::is_deadline_error(&e) => Err(elapsed()),
        Ok(Err(e)) => Err(PluginError::from_lua(method, &e)),
        Err(_) => Err(elapsed()),
    }
}

/// The sign-in wizard gets its own, much longer deadline: a step legitimately
/// waits for the user to finish in their browser.
fn budget(method: &str) -> Duration {
    let wizard = [export::AUTH_BEGIN, export::AUTH_SUBMIT, export::AUTH_CANCEL].contains(&method);
    if wizard {
        super::AUTH_TIMEOUT
    } else {
        super::CALL_TIMEOUT
    }
}

/// Unsupported Lua types are skipped rather than refused, because a plugin's
/// result table often carries its own helper functions next to the data and a
/// stray method must not fail the whole call. Recursive tables still error.
fn deserialize<R: serde::de::DeserializeOwned>(
    lua: &mlua::Lua,
    method: &str,
    value: mlua::Value,
) -> Result<R, PluginError> {
    let options = mlua::serde::DeserializeOptions::new().deny_unsupported_types(false);
    lua.from_value_with(value, options)
        .map_err(|e| PluginError::Runtime {
            method: method.to_string(),
            message: format!("returned something the host could not read: {e}"),
        })
}

fn load_failed(e: impl std::fmt::Display) -> PluginError {
    PluginError::Load(e.to_string())
}

/// The table `setup(ctx)` receives. `api` lets a script that supports several
/// generations branch on the host's rather than refusing to load.
fn setup_ctx(
    lua: &mlua::Lua,
    manifest: &PluginManifest,
    data_dir: &Path,
    locale: &str,
) -> mlua::Result<mlua::Table> {
    let ctx = lua.create_table()?;
    ctx.set("plugin_id", manifest.id.as_str())?;
    ctx.set("data_dir", data_dir.to_string_lossy().as_ref())?;
    ctx.set("locale", locale)?;
    ctx.set("host_version", env!("CARGO_PKG_VERSION"))?;
    ctx.set("api", super::API_VERSION)?;
    Ok(ctx)
}

/// The function-valued keys on the plugin's table. Walked once at load: read on
/// every call, so it must not need the Lua lock or an error path.
fn function_names(table: &mlua::Table) -> HashSet<String> {
    let mut names = HashSet::new();
    let walk = table.for_each::<mlua::Value, mlua::Value>(|key, value| {
        if value.as_function().is_some()
            && let Some(name) = key.as_string()
        {
            names.insert(name.to_string_lossy());
        }
        Ok(())
    });
    if let Err(e) = walk {
        tracing::warn!(error = %e, "cannot read the plugin's exports");
    }
    names
}

/// The language tag handed to the plugin: `de_DE` from `de_DE.UTF-8`, `en` when
/// the environment says nothing.
fn locale() -> String {
    locale_from(std::env::var("LANG").ok().as_deref())
}

fn locale_from(lang: Option<&str>) -> String {
    lang.and_then(|lang| lang.split('.').next())
        .filter(|tag| !tag.is_empty())
        .unwrap_or("en")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_wizard_gets_the_long_deadline() {
        assert_eq!(budget(export::AUTH_BEGIN), super::super::AUTH_TIMEOUT);
        assert_eq!(budget(export::AUTH_SUBMIT), super::super::AUTH_TIMEOUT);
        assert_eq!(budget(export::AUTH_CANCEL), super::super::AUTH_TIMEOUT);
        assert_eq!(budget(export::VALIDATE), super::super::CALL_TIMEOUT);
        assert_eq!(budget(export::SEARCH), super::super::CALL_TIMEOUT);
    }

    #[test]
    fn a_locale_keeps_its_region_and_loses_its_encoding() {
        assert_eq!(locale_from(Some("de_DE.UTF-8")), "de_DE");
        assert_eq!(locale_from(Some("en")), "en");
        assert_eq!(locale_from(Some("")), "en");
        assert_eq!(locale_from(None), "en");
    }
}
