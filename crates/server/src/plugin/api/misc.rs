//! The four leaves of the `kopuz` table that do not deserve a module each:
//! `notify`, `browser`, `time` and `uuid`.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mlua::{Function, Lua, Table};

use super::HostCtx;

/// Ceiling on `kopuz.time.sleep`. Comfortably longer than any sensible retry
/// backoff and shorter than [`CALL_TIMEOUT`](crate::plugin::CALL_TIMEOUT), so a
/// sleep can never be the thing that runs a call out of time on its own.
const MAX_SLEEP_MS: u64 = 60_000;

/// `kopuz.notify.auth_changed(ok)`: tell the host whether the credentials in
/// `kopuz.store` currently work.
///
/// Call it after a sign-in finishes and after any request comes back 401. The host
/// badges the settings row from this, which saves it calling `validate` (and the
/// plugin making a request) just to draw a checkmark. It is a hint, not a promise:
/// the host still calls `validate` when the answer has to be right.
pub(super) fn notify(lua: &Lua, ctx: &Arc<HostCtx>) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    let ctx = Arc::clone(ctx);
    let auth_changed = lua.create_function(move |_, ok: bool| {
        ctx.auth_ok.store(ok, Ordering::Relaxed);
        tracing::debug!(
            target: "plugin",
            plugin = %ctx.plugin_id,
            name = %ctx.plugin_name,
            ok = ok,
            "plugin reported its auth state"
        );
        Ok(())
    })?;
    table.set("auth_changed", auth_changed)?;
    Ok(table)
}

/// `kopuz.browser.open(url)`: hand a URL to the user's browser.
///
/// This exists for the OAuth hop in a sign-in wizard. Prefer returning an
/// [`AuthPrompt::OpenUrl`](crate::plugin::AuthPrompt) step, which lets the host
/// explain what is about to happen before the browser jumps; reach for this when
/// the plugin needs the browser open at some other moment.
///
/// Only `http` and `https` are accepted. Anything else raises
/// `kopuz:invalid_input`, because the platform handler for an arbitrary scheme can
/// launch an arbitrary application.
pub(super) fn browser(lua: &Lua, ctx: &Arc<HostCtx>) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    let ctx = Arc::clone(ctx);
    let open = lua.create_function(move |_, url: mlua::LuaString| {
        let url = super::text_of(&url);
        let parsed = reqwest::Url::parse(&url).map_err(|e| {
            super::invalid_input(format!("browser.open: cannot parse url {url:?}: {e}"))
        })?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(super::invalid_input(format!(
                "browser.open: refusing to open a {:?} url",
                parsed.scheme()
            )));
        }
        webbrowser::open(parsed.as_str()).map_err(|e| {
            tracing::warn!(
                target: "plugin",
                plugin = %ctx.plugin_id,
                error = %e,
                "cannot open a browser"
            );
            mlua::Error::runtime(format!("cannot open a browser: {e}"))
        })
    })?;
    table.set("open", open)?;
    Ok(table)
}

/// `kopuz.time`: `now_ms()` and `sleep(ms)`.
///
/// `now_ms()` is unix milliseconds, for stamping a token expiry into
/// `kopuz.store`. `sleep(ms)` yields the coroutine and is capped at 60000 ms; it
/// is here because a retry needs a backoff and `os.clock` busy-waiting would burn
/// the call's deadline instead of waiting for it.
pub(super) fn time(lua: &Lua, _ctx: &Arc<HostCtx>) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set("now_ms", lua.create_function(|_, (): ()| Ok(now_ms()))?)?;
    table.set("sleep", lua.create_async_function(|_, ms: f64| sleep(ms))?)?;
    Ok(table)
}

/// `kopuz.uuid()`: a random (v4) UUID in the usual hyphenated form.
///
/// For a device id, an OAuth `state` or a request id. Not a substitute for
/// `kopuz.crypto.random_bytes` when a backend asks for entropy of a given length.
pub(super) fn uuid(lua: &Lua, _ctx: &Arc<HostCtx>) -> mlua::Result<Function> {
    lua.create_function(|_, (): ()| Ok(::uuid::Uuid::new_v4().to_string()))
}

/// Unix milliseconds. A clock set before 1970 reports 0 rather than a negative
/// stamp, since every caller is comparing it against a stored expiry.
fn now_ms() -> i64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    i64::try_from(millis).unwrap_or(i64::MAX)
}

async fn sleep(ms: f64) -> mlua::Result<()> {
    tokio::time::sleep(Duration::from_millis(clamp_sleep(ms))).await;
    Ok(())
}

fn clamp_sleep(ms: f64) -> u64 {
    if !ms.is_finite() || ms <= 0.0 {
        return 0;
    }
    (ms as u64).min(MAX_SLEEP_MS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sleep_is_capped_and_never_negative() {
        assert_eq!(clamp_sleep(250.0), 250);
        assert_eq!(clamp_sleep(10_000_000.0), MAX_SLEEP_MS);
        assert_eq!(clamp_sleep(-5.0), 0);
        assert_eq!(clamp_sleep(f64::NAN), 0);
    }

    #[test]
    fn now_ms_is_after_this_code_was_written() {
        // 2024-01-01T00:00:00Z, a floor that catches a clock read that came back
        // as 0 or as seconds instead of milliseconds.
        assert!(now_ms() > 1_704_067_200_000);
    }
}
