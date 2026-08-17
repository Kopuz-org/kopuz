//! `kopuz.json`: the JSON codec, since almost every backend speaks it.
//!
//! How Lua values map to JSON:
//!
//! - A table whose `#` length is non-zero encodes as an array, anything else as
//!   an object. An empty table is therefore `{}`, never `[]`.
//! - A table that came out of [`decode`] as an array carries mlua's array marker,
//!   so decoding and re-encoding preserves arrays, empty ones included.
//! - `nil` inside a table is an absent key, exactly as in Lua. Use
//!   `kopuz.json.null` for an explicit JSON `null`.
//! - Functions, coroutines and userdata are rejected rather than silently
//!   skipped, so a typo in a table literal fails loudly.

use std::sync::Arc;

use mlua::{Lua, LuaSerdeExt, Table, Value};

use super::HostCtx;

/// `encode(value) -> string`, `decode(string) -> value`, and the `null` sentinel.
///
/// ```lua
/// local body = kopuz.json.encode({ ids = { 1, 2, 3 }, cursor = kopuz.json.null })
/// local parsed = kopuz.json.decode(body)
/// if parsed.cursor == kopuz.json.null then … end
/// ```
///
/// Both directions raise `kopuz:invalid_input` on input they cannot handle.
pub(super) fn module(lua: &Lua, _ctx: &Arc<HostCtx>) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set(
        "encode",
        lua.create_function(|lua, value: Value| encode(lua, value))?,
    )?;
    table.set(
        "decode",
        lua.create_function(|lua, text: mlua::LuaString| decode(lua, &super::bytes_of(&text)))?,
    )?;
    // A distinct sentinel rather than `nil`: a Lua table cannot hold nil, so
    // without this a plugin could neither read nor write an explicit JSON null.
    table.set("null", lua.null())?;
    Ok(table)
}

/// Serialize a Lua value to a compact JSON string.
pub(super) fn encode(lua: &Lua, value: Value) -> mlua::Result<String> {
    let json: serde_json::Value = lua
        .from_value(value)
        .map_err(|e| super::invalid_input(format!("cannot encode as JSON: {e}")))?;
    serde_json::to_string(&json)
        .map_err(|e| super::invalid_input(format!("cannot encode as JSON: {e}")))
}

/// Parse JSON bytes into a Lua value. Shared with `kopuz.http`, whose response
/// `json()` method is this function over the response body.
pub(super) fn decode(lua: &Lua, bytes: &[u8]) -> mlua::Result<Value> {
    let json: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|e| super::invalid_input(format!("malformed JSON: {e}")))?;
    lua.to_value(&json)
}
