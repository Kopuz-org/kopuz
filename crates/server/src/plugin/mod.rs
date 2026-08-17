//! Lua plugin runtime.
//!
//! A plugin is a directory with a `plugin.toml` and a Lua entry chunk. The host
//! loads the chunk in a sandboxed [`mlua`] state, calls its `setup(ctx)` for the
//! handshake, and from then on drives it by calling the functions on the table
//! the chunk returned. [`crate::source::plugin::PluginSource`] is the
//! [`MediaSource`](crate::source::MediaSource) that turns those calls into the
//! app's own operations.
//!
//! Why in-process Lua and not a child process: a plugin is a few hundred lines of
//! glue against one HTTP API. As a binary, every author had to pick a language, a
//! JSON-RPC library and a build toolchain before writing a single request, and
//! shipping one meant shipping a per-platform artifact. A Lua file is portable by
//! construction, and the host hands it the batteries (`http`, `json`, `crypto`,
//! `store`) instead of watching it reimplement them.
//!
//! What keeps that safe: the state loads only the safe stdlib subset with `io`,
//! `os.execute` and the C-module loader stripped, `require` resolves only inside
//! the plugin's own directory, allocation is capped at [`MEMORY_LIMIT`], and every
//! call carries a deadline enforced from an instruction hook as well as by the
//! async timeout, so `while true do end` fails one call instead of wedging the
//! runtime. What it is *not* is a security boundary against a hostile plugin:
//! `http` reaches the network and `store` reaches a directory, so installing a
//! plugin is still trusting its author.
//!
//! Nothing outside this module knows the plugin is Lua. The seam is
//! [`PluginInstance::call`] plus the tables in [`dto`].

use std::borrow::Cow;

use crate::source::SourceError;

mod api;
mod auth;
mod dto;
mod instance;
mod lua;
pub mod manifest;
mod registry;

pub use auth::{auth_begin, auth_cancel, auth_submit};
pub use dto::*;
pub use instance::PluginInstance;
pub use manifest::{API_VERSION, PluginManifest};
pub use registry::{PluginRegistry, registry, shutdown_all};

/// How long an ordinary plugin call may run before it is failed. Covers waiting
/// on the network and burning CPU in Lua alike: the deadline is checked from the
/// instruction hook as well as by the async timeout, so neither a hung request
/// nor a spin loop can outlive it.
pub const CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// The sign-in wizard's own, much longer deadline. A step legitimately sits
/// waiting for the user to finish an OAuth round trip in their browser.
pub const AUTH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// Allocation ceiling for one plugin's Lua state. Generous for glue code, low
/// enough that a runaway table allocation fails the plugin and not the app.
pub const MEMORY_LIMIT: usize = 256 * 1024 * 1024;

/// How often the instruction hook checks the deadline. Small enough to catch a
/// spin loop promptly, large enough not to show up in a profile.
pub const HOOK_INTERVAL: u32 = 200_000;

/// How many results `search` is asked for. The plugin is free to return fewer.
pub const SEARCH_LIMIT: u32 = 100;

/// The prefix `kopuz.fail(code, message)` puts on the Lua error it raises, so a
/// classified failure survives the trip through Lua's plain-string errors. A
/// plain `error("boom")` has no prefix and lands as [`PluginError::Runtime`].
pub(crate) const FAIL_PREFIX: &str = "kopuz:";

/// Why a plugin call failed.
///
/// Mapped to [`SourceError`] at the [`MediaSource`](crate::source::MediaSource)
/// boundary, but kept separate from it so the runtime can tell "this plugin is
/// not installed" from "this source cannot do that", and the UI treats those
/// differently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginError {
    /// The plugin's table has no such function. Not a malfunction: it is how a
    /// plugin declines an optional operation.
    Unsupported(String),
    /// No discoverable manifest with this id.
    NotInstalled(String),
    /// The entry chunk would not load, or `setup` failed. The plugin is unusable
    /// until it is fixed and rescanned.
    Load(String),
    /// The Lua function raised, or handed back a table the host could not read.
    Runtime { method: String, message: String },
    /// The call outlived its deadline.
    Timeout { method: String },
    /// The plugin reported the user is not signed in (`kopuz.fail("auth", …)`).
    Auth,
    /// The plugin could not reach its backend (`kopuz.fail("connectivity", …)`).
    Connectivity,
    /// The plugin rejected the arguments (`kopuz.fail("invalid_input", …)`).
    InvalidInput(String),
}

impl PluginError {
    /// Classify a raised Lua error. `kopuz.fail` writes `kopuz:<code>: <message>`;
    /// anything else is an ordinary runtime failure.
    pub(crate) fn from_lua(method: &str, err: &mlua::Error) -> Self {
        let text = err.to_string();
        let runtime = || Self::Runtime {
            method: method.to_string(),
            message: text.clone(),
        };
        let Some(rest) = find_fail_marker(&text) else {
            return runtime();
        };
        let (code, message) = match rest.split_once(':') {
            Some((code, message)) => (code.trim(), message.trim().to_string()),
            None => (rest.trim(), String::new()),
        };
        match code {
            api::code::AUTH => Self::Auth,
            api::code::CONNECTIVITY => Self::Connectivity,
            api::code::INVALID_INPUT => Self::InvalidInput(message),
            api::code::UNSUPPORTED => Self::Unsupported(if message.is_empty() {
                method.to_string()
            } else {
                message
            }),
            _ => runtime(),
        }
    }
}

/// Lua stamps its own `chunk:line:` context onto an error string, so the marker
/// is looked for anywhere in the message rather than only at the front. The
/// message itself is truncated at the first newline: Lua appends a traceback.
fn find_fail_marker(text: &str) -> Option<&str> {
    let rest = &text[text.find(FAIL_PREFIX)? + FAIL_PREFIX.len()..];
    Some(rest.split('\n').next().unwrap_or(rest))
}

impl std::fmt::Display for PluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported(op) => write!(f, "the plugin does not implement {op}"),
            Self::NotInstalled(id) => write!(f, "plugin {id} is not installed"),
            Self::Load(reason) => write!(f, "plugin failed to load: {reason}"),
            Self::Runtime { method, message } => write!(f, "{method}: {message}"),
            Self::Timeout { method } => write!(f, "{method} timed out"),
            Self::Auth => f.write_str("the plugin is not signed in"),
            Self::Connectivity => f.write_str("the plugin cannot reach its backend"),
            Self::InvalidInput(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for PluginError {}

impl From<PluginError> for SourceError {
    fn from(e: PluginError) -> Self {
        match e {
            PluginError::Unsupported(op) => Self::Unsupported(Cow::Owned(op)),
            PluginError::Auth => Self::Auth,
            PluginError::Connectivity => Self::Connectivity,
            PluginError::InvalidInput(m) => Self::InvalidInput(m),
            other => Self::Backend(other.to_string()),
        }
    }
}

/// Split a namespaced item id (`"<plugin_id>/<item_ref>"`) into its parts.
/// Returns `None` when the id carries no plugin prefix.
pub fn split_item_id(item_id: &str) -> Option<(&str, &str)> {
    let (plugin_id, rest) = item_id.split_once('/')?;
    (manifest::is_valid_id(plugin_id) && !rest.is_empty()).then_some((plugin_id, rest))
}

/// Namespace a plugin-supplied item ref so two plugins can never collide in the
/// database.
pub fn namespace_item_id(plugin_id: &str, item_ref: &str) -> String {
    format!("{plugin_id}/{item_ref}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lua_err(message: &str) -> mlua::Error {
        mlua::Error::RuntimeError(message.to_string())
    }

    #[test]
    fn item_ids_round_trip() {
        let id = namespace_item_id("example", "track-1");
        assert_eq!(id, "example/track-1");
        assert_eq!(split_item_id(&id), Some(("example", "track-1")));
    }

    #[test]
    fn unprefixed_ids_are_rejected() {
        assert_eq!(split_item_id("bare"), None);
        assert_eq!(split_item_id("example/"), None);
        assert_eq!(split_item_id("BAD/x"), None);
    }

    #[test]
    fn classifies_fail_codes() {
        assert_eq!(
            PluginError::from_lua("validate", &lua_err("init.lua:4: kopuz:auth: expired")),
            PluginError::Auth
        );
        assert_eq!(
            PluginError::from_lua("search", &lua_err("kopuz:connectivity: dns")),
            PluginError::Connectivity
        );
        assert_eq!(
            PluginError::from_lua("search", &lua_err("kopuz:invalid_input: empty query")),
            PluginError::InvalidInput("empty query".into())
        );
    }

    #[test]
    fn a_traceback_does_not_leak_into_the_message() {
        assert_eq!(
            PluginError::from_lua(
                "search",
                &lua_err("init.lua:4: kopuz:invalid_input: bad\nstack traceback:\n\t[C]: in ?")
            ),
            PluginError::InvalidInput("bad".into())
        );
    }

    #[test]
    fn unsupported_defaults_to_the_method_name() {
        assert_eq!(
            PluginError::from_lua("start_radio", &lua_err("kopuz:unsupported")),
            PluginError::Unsupported("start_radio".into())
        );
    }

    /// The message keeps whatever mlua put in front of it (`runtime error:`,
    /// `memory error:`), because which kind of failure it was is the first thing
    /// a plugin author needs and the host cannot recover it later.
    #[test]
    fn a_plain_error_stays_a_runtime_failure() {
        let err = PluginError::from_lua("search", &lua_err("init.lua:9: boom"));
        assert!(matches!(err, PluginError::Runtime { .. }));
        assert_eq!(
            SourceError::from(err),
            SourceError::Backend("search: runtime error: init.lua:9: boom".into())
        );
    }
}
