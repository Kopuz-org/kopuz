//! The `kopuz` global: every host capability a plugin can reach.
//!
//! A plugin's Lua state has no `io`, no `os.execute` and no C-module loader, so
//! everything it needs to talk to a music backend arrives through this one
//! table. The surface is deliberately narrow: requests, JSON, digests,
//! percent-encoding, a key/value store, logging and a clock. Anything a plugin
//! could reasonably write in pure Lua is not here.
//!
//! Each submodule owns one sub-table and doubles as its reference
//! documentation, since a plugin author reads these doc comments next to
//! `docs/plugins.md`. Adding, removing or reshaping anything here is a breaking
//! change and needs [`API_VERSION`](manifest::API_VERSION) bumped.
//!
//! ```lua
//! local res = kopuz.http.get("https://example.test/api/tracks", {
//!   headers = { authorization = "Bearer " .. kopuz.store.get("token") },
//!   query = { limit = 50 },
//! })
//! if res.status == 401 then
//!   kopuz.fail("auth", "token expired")
//! end
//! return res:json()
//! ```

use std::sync::Arc;

use mlua::Lua;

use crate::plugin::{FAIL_PREFIX, manifest};

mod crypto;
mod http;
mod json;
mod log;
mod misc;
mod store;
mod url;

/// The classified failure codes `kopuz.fail` accepts.
///
/// These are the only strings the host turns into a specific
/// [`PluginError`](crate::plugin::PluginError); anything else still fails the
/// call, but as an unclassified malfunction, which the UI reports as a bug
/// rather than as something the user can act on.
pub(super) mod code {
    /// The user is not signed in, or their credentials expired. Prompts the UI
    /// to offer the sign-in wizard again.
    pub const AUTH: &str = "auth";
    /// The backend could not be reached. Kept apart from `auth` because the UI
    /// must not tell someone to sign in again when their wifi is off.
    pub const CONNECTIVITY: &str = "connectivity";
    /// The arguments were unusable. The message reaches the user verbatim.
    pub const INVALID_INPUT: &str = "invalid_input";
    /// This plugin does not implement the operation. Rarely needed by hand: a
    /// function the plugin simply does not export is already unsupported.
    pub const UNSUPPORTED: &str = "unsupported";
}

/// What the host knows about the plugin it is installing this table for.
pub(super) struct HostCtx {
    pub plugin_id: String,
    pub plugin_name: String,
    pub data_dir: std::path::PathBuf,
    pub locale: String,
    /// Flipped by `kopuz.notify.auth_changed(ok)`; read by the host to badge the
    /// settings row without calling `validate`.
    pub auth_ok: std::sync::atomic::AtomicBool,
}

/// Build the `kopuz` table and set it as a global on `lua`.
pub(super) fn install(lua: &Lua, ctx: Arc<HostCtx>) -> mlua::Result<()> {
    let kopuz = lua.create_table()?;

    // Scalars, read once during `setup` in practice.
    kopuz.set("version", env!("CARGO_PKG_VERSION"))?;
    kopuz.set("api", manifest::API_VERSION)?;
    kopuz.set("plugin_id", ctx.plugin_id.clone())?;
    kopuz.set("data_dir", ctx.data_dir.to_string_lossy().into_owned())?;
    kopuz.set("locale", ctx.locale.clone())?;

    kopuz.set("fail", fail_function(lua)?)?;

    kopuz.set("log", log::module(lua, &ctx)?)?;
    kopuz.set("http", http::module(lua, &ctx)?)?;
    kopuz.set("json", json::module(lua, &ctx)?)?;
    kopuz.set("crypto", crypto::module(lua, &ctx)?)?;
    kopuz.set("url", url::module(lua, &ctx)?)?;
    kopuz.set("store", store::module(lua, &ctx)?)?;

    // `misc` holds four unrelated leaves rather than one sub-table, so they are
    // spelled out here instead of hidden behind a merge.
    kopuz.set("notify", misc::notify(lua, &ctx)?)?;
    kopuz.set("browser", misc::browser(lua, &ctx)?)?;
    kopuz.set("time", misc::time(lua, &ctx)?)?;
    kopuz.set("uuid", misc::uuid(lua, &ctx)?)?;

    lua.globals().set("kopuz", kopuz)
}

/// `kopuz.fail(code, message)`: raise a failure the host can classify.
///
/// `code` is one of `"auth"`, `"connectivity"`, `"invalid_input"` or
/// `"unsupported"` (see [`code`]); `message` is optional. This is the only way a
/// plugin can say *why* something went wrong, so prefer it over a bare
/// `error()`, which the host can only report as a malfunction.
///
/// ```lua
/// kopuz.fail("auth", "refresh token rejected")
/// ```
fn fail_function(lua: &Lua) -> mlua::Result<mlua::Function> {
    lua.create_function(
        |_, (code, message): (mlua::LuaString, Option<mlua::LuaString>)| -> mlua::Result<()> {
            let code = text_of(&code);
            let code = code.trim();
            // The host splits on the first ':' after the prefix, so a code
            // carrying one would arrive truncated with the rest of it prepended
            // to the message.
            if code.is_empty() || code.contains(':') {
                return Err(invalid_input(
                    "fail() code must be non-empty and contain no ':'",
                ));
            }
            let message = message.as_ref().map(text_of).unwrap_or_default();
            Err(fail(code, message))
        },
    )
}

/// Build a classified failure in the wire shape
/// [`PluginError::from_lua`](crate::plugin::PluginError::from_lua) parses.
///
/// The message is flattened onto one line: Lua appends a traceback to whatever
/// it raises and the host truncates at the first newline, so an embedded newline
/// would silently swallow the rest of the message.
pub(super) fn fail(code: &str, message: impl std::fmt::Display) -> mlua::Error {
    let message = message.to_string();
    let flat = message.split_whitespace().collect::<Vec<_>>().join(" ");
    mlua::Error::runtime(format!("{FAIL_PREFIX}{code}: {flat}"))
}

/// `kopuz.fail("invalid_input", …)` raised from the host side.
pub(super) fn invalid_input(message: impl std::fmt::Display) -> mlua::Error {
    fail(code::INVALID_INPUT, message)
}

/// `kopuz.fail("connectivity", …)` raised from the host side.
pub(super) fn connectivity(message: impl std::fmt::Display) -> mlua::Error {
    fail(code::CONNECTIVITY, message)
}

/// A Lua string's bytes, copied out at once: the borrow holds the interpreter
/// lock, which must never be alive across an `.await`.
pub(super) fn bytes_of(s: &mlua::LuaString) -> Vec<u8> {
    s.as_bytes().to_vec()
}

/// A Lua string as text, replacing invalid UTF-8. Lua strings are byte strings,
/// and refusing a stray byte in a log line or a header value is never the more
/// useful answer.
pub(super) fn text_of(s: &mlua::LuaString) -> String {
    String::from_utf8_lossy(&s.as_bytes()).into_owned()
}

/// One of Lua's scalars as text, for the string-keyed string-valued tables
/// `kopuz.http` and `kopuz.url` take. Booleans are allowed because
/// `{ explicit = true }` is a natural thing to write in a query table; anything
/// richer than a scalar is a mistake worth reporting.
pub(super) fn scalar_text(value: &mlua::Value, what: &str) -> mlua::Result<String> {
    match value {
        mlua::Value::String(s) => Ok(text_of(s)),
        mlua::Value::Integer(i) => Ok(i.to_string()),
        mlua::Value::Number(n) => Ok(n.to_string()),
        mlua::Value::Boolean(b) => Ok(b.to_string()),
        other => Err(invalid_input(format!(
            "{what} must be a string, number or boolean, got {}",
            other.type_name()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::PluginError;

    #[test]
    fn fail_round_trips_through_the_host_classifier() {
        let err = fail(code::AUTH, "token expired");
        assert_eq!(PluginError::from_lua("validate", &err), PluginError::Auth);
    }

    #[test]
    fn a_multiline_message_keeps_its_tail() {
        let err = fail(code::INVALID_INPUT, "first line\nsecond line");
        assert_eq!(
            PluginError::from_lua("search", &err),
            PluginError::InvalidInput("first line second line".into())
        );
    }
}
