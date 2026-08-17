//! `kopuz.log`: diagnostics that land in the app's own log.
//!
//! Everything is re-emitted through `tracing` at target `plugin` with the plugin
//! id as a field, so a user filing a bug report captures plugin output alongside
//! the rest of the app and the plugin never needs a file to write to. The level
//! decides whether it survives the default filter: `info` and up show, `debug`
//! and `trace` need `RUST_LOG` widened.
//!
//! Values are stringified the way `tostring` would, so a table logs as
//! `table: 0x…`. Log `kopuz.json.encode(t)` when you want to see inside one.

use std::sync::Arc;

use mlua::{Lua, Table, Value};
use tracing::Level;

use super::HostCtx;

/// `trace`, `debug`, `info`, `warn` and `error`, each taking one value.
///
/// ```lua
/// kopuz.log.info("signed in as " .. account)
/// kopuz.log.debug(kopuz.json.encode(response_body))
/// ```
pub(super) fn module(lua: &Lua, ctx: &Arc<HostCtx>) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    for (name, level) in [
        ("trace", Level::TRACE),
        ("debug", Level::DEBUG),
        ("info", Level::INFO),
        ("warn", Level::WARN),
        ("error", Level::ERROR),
    ] {
        let ctx = Arc::clone(ctx);
        let f = lua.create_function(move |_, value: Value| {
            emit(level, &ctx.plugin_id, &stringify(&value));
            Ok(())
        })?;
        table.set(name, f)?;
    }
    Ok(table)
}

/// `tracing` builds its callsite metadata at compile time, so the level cannot
/// come from a variable and each one needs its own macro call.
fn emit(level: Level, plugin: &str, message: &str) {
    match level {
        Level::TRACE => tracing::trace!(target: "plugin", plugin = %plugin, "{message}"),
        Level::DEBUG => tracing::debug!(target: "plugin", plugin = %plugin, "{message}"),
        Level::INFO => tracing::info!(target: "plugin", plugin = %plugin, "{message}"),
        Level::WARN => tracing::warn!(target: "plugin", plugin = %plugin, "{message}"),
        Level::ERROR => tracing::error!(target: "plugin", plugin = %plugin, "{message}"),
    }
}

/// `tostring` semantics, with a lossy fallback for a byte string that is not
/// valid UTF-8 and for a `__tostring` that raises.
fn stringify(value: &Value) -> String {
    if let Value::String(s) = value {
        return super::text_of(s);
    }
    value
        .to_string()
        .unwrap_or_else(|_| format!("<{}>", value.type_name()))
}
