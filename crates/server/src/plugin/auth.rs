//! The sign-in wizard, as the settings page sees it.
//!
//! Three async functions and a prompt enum, so the UI never touches an
//! [`mlua`] type or knows a step is a Lua call. The loop is the caller's: begin,
//! render whatever [`AuthPrompt`] comes back, submit what the user typed, repeat
//! until [`AuthPrompt::Done`] or [`AuthPrompt::Failed`].
//!
//! Failures arrive as the error string rather than as a `Failed` prompt so the
//! caller can decide whether the wizard stays open; in practice it renders both
//! the same way.

use super::dto::export;
use super::{AuthPrompt, AuthValues, PluginError, registry};

/// Start the wizard. A plugin with nothing to sign into does not export
/// `auth_begin`, and that answers [`Done`](AuthPrompt::Done) immediately.
pub async fn auth_begin(plugin_id: &str) -> Result<AuthPrompt, String> {
    step(plugin_id, export::AUTH_BEGIN, ()).await
}

/// Post the values the user entered. Empty for an `open_url` or `message` step,
/// where submitting is just the user saying they are done.
pub async fn auth_submit(plugin_id: &str, values: AuthValues) -> Result<AuthPrompt, String> {
    step(plugin_id, export::AUTH_SUBMIT, values).await
}

/// Tell the plugin the user backed out. Best effort: the wizard is closing
/// either way, so a plugin that does not care simply does not export this.
pub async fn auth_cancel(plugin_id: &str) {
    let Ok(instance) = registry().instance(plugin_id).await else {
        return;
    };
    if let Err(e) = instance.call_unit(export::AUTH_CANCEL, ()).await
        && !matches!(e, PluginError::Unsupported(_))
    {
        tracing::warn!(plugin = plugin_id, error = %e, "auth_cancel failed");
    }
}

async fn step<A>(plugin_id: &str, method: &'static str, args: A) -> Result<AuthPrompt, String>
where
    A: mlua::IntoLuaMulti + Send + 'static,
{
    let instance = registry()
        .instance(plugin_id)
        .await
        .map_err(|e| e.to_string())?;
    match instance.call::<A, AuthPrompt>(method, args).await {
        Ok(prompt) => Ok(prompt),
        Err(PluginError::Unsupported(_)) => Ok(AuthPrompt::Done),
        Err(e) => Err(e.to_string()),
    }
}
