//! The sandboxed Lua state a plugin runs in, and the two things that keep it
//! honest: a `require` that resolves only inside the plugin's own directory, and
//! a deadline the instruction hook enforces from inside the VM.
//!
//! [`build`] is the only way a state is created, so every plugin gets the same
//! stdlib subset, the same allocation ceiling and the same hook. What it is not
//! is a security boundary: see the module docs in [`super`].

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use mlua::{HookTriggers, Lua, LuaOptions, StdLib, Value, VmState};

use super::PluginError;
use super::manifest::{self, PluginManifest};

/// A plugin's Lua state plus the clock its calls run against.
pub(super) struct Sandbox {
    pub lua: Lua,
    /// Unix millis the currently running call must finish by. 0 = no call in
    /// flight. Read by the instruction hook.
    pub deadline: Arc<AtomicI64>,
}

impl Sandbox {
    /// Start the clock for one call. The returned guard clears the deadline again,
    /// so an idle state never trips the hook, and so an early `?` cannot leave a
    /// stale deadline behind to fail the next call.
    pub fn arm(&self, timeout: Duration) -> DeadlineGuard<'_> {
        let budget = i64::try_from(timeout.as_millis()).unwrap_or(i64::MAX);
        self.deadline
            .store(now_millis().saturating_add(budget), Ordering::Relaxed);
        DeadlineGuard(&self.deadline)
    }
}

pub(super) struct DeadlineGuard<'a>(&'a AtomicI64);

impl Drop for DeadlineGuard<'_> {
    fn drop(&mut self) {
        self.0.store(0, Ordering::Relaxed);
    }
}

/// Build a plugin's state: safe stdlib subset, capped memory, deadline hook,
/// contained `require`, and the `kopuz` global.
pub(super) fn build(
    manifest: &PluginManifest,
    host: Arc<super::api::HostCtx>,
) -> Result<Sandbox, PluginError> {
    assemble(manifest, host).map_err(|e| PluginError::Load(e.to_string()))
}

fn assemble(manifest: &PluginManifest, host: Arc<super::api::HostCtx>) -> mlua::Result<Sandbox> {
    // `io`, `debug`, `package` and `ffi` are simply never opened. Leaving
    // `package` out is also what makes a custom `require` necessary.
    let libs = StdLib::TABLE
        | StdLib::STRING
        | StdLib::MATH
        | StdLib::COROUTINE
        | StdLib::OS
        | StdLib::UTF8;
    let lua = Lua::new_with(libs, LuaOptions::default())?;
    lua.set_memory_limit(super::MEMORY_LIMIT)?;
    strip_globals(&lua)?;

    let deadline = Arc::new(AtomicI64::new(0));
    install_deadline_hook(&lua, Arc::clone(&deadline))?;
    install_require(&lua, manifest.dir.clone())?;
    super::api::install(&lua, host)?;

    Ok(Sandbox { lua, deadline })
}

/// What is left to take away once the unsafe libraries are unloaded: the `os`
/// entries that reach the process or the filesystem, and the loaders that would
/// let a plugin run code from anywhere but its own directory.
fn strip_globals(lua: &Lua) -> mlua::Result<()> {
    let globals = lua.globals();
    let os = globals.raw_get::<mlua::Table>("os")?;
    for name in [
        "execute",
        "exit",
        "remove",
        "rename",
        "tmpname",
        "setlocale",
        "getenv",
    ] {
        os.raw_set(name, Value::Nil)?;
    }
    // `loadstring` is 5.1 only; setting a key that was never there is a no-op.
    for name in ["dofile", "loadfile", "load", "loadstring"] {
        globals.raw_set(name, Value::Nil)?;
    }
    Ok(())
}

/// The hook is global rather than per-thread because every call runs in a
/// coroutine mlua creates for it, and a hook set on one thread does not follow
/// into the next.
fn install_deadline_hook(lua: &Lua, deadline: Arc<AtomicI64>) -> mlua::Result<()> {
    lua.set_global_hook(
        HookTriggers::new().every_nth_instruction(super::HOOK_INTERVAL),
        move |_, _| {
            let due = deadline.load(Ordering::Relaxed);
            if due != 0 && now_millis() > due {
                return Err(deadline_error());
            }
            Ok(VmState::Continue)
        },
    )
}

/// Unix millis, 0 if the clock is somehow before the epoch. The hook and the
/// caller arming the deadline share this, so a broken clock cannot make one of
/// them see a deadline the other does not.
fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|since| i64::try_from(since.as_millis()).ok())
        .unwrap_or(0)
}

/// Shaped like a `kopuz.fail` so it survives Lua's string-only errors, and
/// recognised again by [`is_deadline_error`] because `timeout` is not one of the
/// codes [`PluginError::from_lua`] classifies.
fn deadline_error() -> mlua::Error {
    mlua::Error::runtime(format!(
        "{}timeout: call exceeded its deadline",
        super::FAIL_PREFIX
    ))
}

/// Whether a raised error is the hook's. Plugin code is free to `pcall` around
/// it and keep running, which is why the async timeout exists as well.
pub(super) fn is_deadline_error(err: &mlua::Error) -> bool {
    err.to_string()
        .contains(&format!("{}timeout", super::FAIL_PREFIX))
}

/// `require("lib.http")` reads `<plugin dir>/lib/http.lua`, once. Async because a
/// module's top level is allowed to call the host, and a host call yields.
fn install_require(lua: &Lua, dir: PathBuf) -> mlua::Result<()> {
    let cache = lua.create_table()?;
    let require = lua.create_async_function(move |lua, name: String| {
        load_module(lua, cache.clone(), dir.clone(), name)
    })?;
    lua.globals().raw_set("require", require)
}

async fn load_module(
    lua: Lua,
    cache: mlua::Table,
    dir: PathBuf,
    name: String,
) -> mlua::Result<Value> {
    if let Some(loaded) = cache.raw_get::<Option<Value>>(name.as_str())? {
        return Ok(loaded);
    }
    let rel = module_path(&name).ok_or_else(|| {
        mlua::Error::runtime(format!(
            "require {name:?}: a module must resolve inside the plugin directory"
        ))
    })?;
    let body = std::fs::read_to_string(dir.join(&rel)).map_err(|e| {
        mlua::Error::runtime(format!(
            "require {name:?}: cannot read {}: {e}",
            rel.display()
        ))
    })?;
    let value: Value = lua
        .load(body)
        .set_name(format!("@{}", rel.display()))
        .eval_async()
        .await?;
    // Lua's own semantics: a chunk that returns nothing still counts as loaded,
    // so requiring it again does not run it again.
    let value = if value.is_nil() {
        Value::Boolean(true)
    } else {
        value
    };
    cache.raw_set(name.as_str(), value.clone())?;
    Ok(value)
}

/// `lib.http` to `lib/http.lua`, and nothing that could point outside the plugin
/// directory. `..` is rejected before the dots become separators, and a
/// backslash is rejected outright because it separates on Windows while being an
/// ordinary character to a unix `Path`.
fn module_path(name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.contains("..") || name.starts_with('/') || name.contains('\\') {
        return None;
    }
    let rel = PathBuf::from(format!("{}.lua", name.replace('.', "/")));
    manifest::is_contained_relative(&rel).then_some(rel)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        root: PathBuf,
        sandbox: Sandbox,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.root).ok();
        }
    }

    fn fixture(files: &[(&str, &str)]) -> Fixture {
        let root = std::env::temp_dir().join(format!("kopuz-lua-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("mkdir");
        for (name, body) in files {
            let path = root.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("mkdir");
            }
            std::fs::write(&path, body).expect("write module");
        }

        let manifest = PluginManifest {
            id: "test".to_string(),
            name: "Test".to_string(),
            version: "1.0".to_string(),
            api: manifest::API_VERSION,
            entry: PathBuf::from("init.lua"),
            icon: None,
            accent: None,
            dir: root.clone(),
        };
        let host = Arc::new(super::super::api::HostCtx {
            plugin_id: manifest.id.clone(),
            plugin_name: manifest.name.clone(),
            data_dir: root.join("data"),
            locale: "en".to_string(),
            auth_ok: std::sync::atomic::AtomicBool::new(false),
        });
        let sandbox = build(&manifest, host).expect("sandbox builds");
        Fixture { root, sandbox }
    }

    #[test]
    fn the_dangerous_stdlib_is_not_there() {
        let f = fixture(&[]);
        let stripped: bool = f
            .sandbox
            .lua
            .load(
                "return io == nil and package == nil and debug == nil \
                 and load == nil and loadfile == nil and dofile == nil \
                 and os.execute == nil and os.getenv == nil and os.remove == nil \
                 and os.time ~= nil and os.date ~= nil",
            )
            .eval()
            .expect("eval");
        assert!(stripped);
    }

    #[tokio::test]
    async fn require_loads_a_sibling_and_caches_it() {
        let f = fixture(&[(
            "lib/http.lua",
            r#"return { ping = function() return "pong" end }"#,
        )]);
        let pong: String = f
            .sandbox
            .lua
            .load(r#"local m = require("lib.http") return m.ping()"#)
            .eval_async()
            .await
            .expect("eval");
        assert_eq!(pong, "pong");

        let same: bool = f
            .sandbox
            .lua
            .load(r#"return require("lib.http") == require("lib.http")"#)
            .eval_async()
            .await
            .expect("eval");
        assert!(same, "a second require must hand back the same module");
    }

    #[tokio::test]
    async fn require_refuses_to_leave_the_plugin_directory() {
        let f = fixture(&[]);
        for name in ["../escape", "..", "/etc/passwd", "lib\\escape", ""] {
            let result = f
                .sandbox
                .lua
                .load(format!("return require([[{name}]])"))
                .eval_async::<Value>()
                .await;
            assert!(result.is_err(), "require({name:?}) must fail");
        }
    }

    #[test]
    fn a_deadline_stops_a_spin_loop() {
        let f = fixture(&[]);
        let armed = f.sandbox.arm(Duration::from_millis(20));
        let err = f
            .sandbox
            .lua
            .load("while true do end")
            .exec()
            .expect_err("the loop must be cut off");
        assert!(is_deadline_error(&err), "unexpected error: {err}");

        drop(armed);
        f.sandbox
            .lua
            .load("for _ = 1, 100000 do end")
            .exec()
            .expect("a cleared deadline never fires");
    }
}
