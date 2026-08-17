//! `kopuz.store`: the plugin's own persisted key/value state.
//!
//! Kopuz holds no credentials of its own for a plugin. It never learns what a
//! plugin's backend needs, so there is nothing for it to keep: whatever comes out
//! of the sign-in wizard is the plugin's to store here, and `validate` reads it
//! back on the next launch. That also means a plugin that stores nothing is
//! signed out the moment the app restarts, which is a choice a plugin author has
//! to make deliberately.
//!
//! The file is `<data_dir>/store.json`, plain JSON and **not encrypted**. It sits
//! in the user's config directory with the rest of the app's state, under the same
//! filesystem permissions, and anything that can read the app's database can read
//! it too. Treat it as "as private as the rest of Kopuz", not as a keychain.
//!
//! Keys are opaque strings with no path semantics. A key containing `/` is a
//! perfectly ordinary key and never becomes a directory: this is a map, not a
//! filesystem.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use mlua::{Lua, LuaSerdeExt, Table, Value};

use super::HostCtx;

const FILE_NAME: &str = "store.json";

/// `get`, `set`, `delete`, `keys` and `clear`.
///
/// | function | result |
/// | --- | --- |
/// | `get(key)` | the stored value, or `nil` |
/// | `set(key, value)` | any JSON-encodable value. `nil` deletes, the way it does in a Lua table; use `kopuz.json.null` to store a real null |
/// | `delete(key)` | removes the key, whether or not it was there |
/// | `keys()` | every key, sorted |
/// | `clear()` | empties the store |
///
/// ```lua
/// kopuz.store.set("session", { token = token, expires_at = kopuz.time.now_ms() + 3600000 })
/// local session = kopuz.store.get("session")
/// ```
///
/// Every mutation is written through to disk before it returns, and the write is
/// atomic, so a crash cannot leave a half-written store behind. A failed write
/// raises: a plugin must not carry on believing it saved a token it did not.
pub(super) fn module(lua: &Lua, ctx: &Arc<HostCtx>) -> mlua::Result<Table> {
    let state = Arc::new(Mutex::new(Store::new(ctx.data_dir.join(FILE_NAME))));
    let table = lua.create_table()?;

    let shared = Arc::clone(&state);
    table.set(
        "get",
        lua.create_function(move |lua, key: mlua::LuaString| {
            let key = super::text_of(&key);
            let mut store = lock(&shared);
            match store.entries().get(&key) {
                Some(value) => lua.to_value(value),
                None => Ok(Value::Nil),
            }
        })?,
    )?;

    let shared = Arc::clone(&state);
    table.set(
        "set",
        lua.create_function(move |lua, (key, value): (mlua::LuaString, Value)| {
            let key = super::text_of(&key);
            let mut store = lock(&shared);
            if value.is_nil() {
                store.entries().remove(&key);
            } else {
                let json: serde_json::Value = lua.from_value(value).map_err(|e| {
                    super::invalid_input(format!("store.set({key:?}): cannot store value: {e}"))
                })?;
                store.entries().insert(key, json);
            }
            persist(&store)
        })?,
    )?;

    let shared = Arc::clone(&state);
    table.set(
        "delete",
        lua.create_function(move |_, key: mlua::LuaString| {
            let key = super::text_of(&key);
            let mut store = lock(&shared);
            store.entries().remove(&key);
            persist(&store)
        })?,
    )?;

    let shared = Arc::clone(&state);
    table.set(
        "keys",
        lua.create_function(move |lua, (): ()| {
            let mut store = lock(&shared);
            let out = lua.create_table()?;
            for key in store.entries().keys() {
                out.push(key.as_str())?;
            }
            Ok(out)
        })?,
    )?;

    let shared = Arc::clone(&state);
    table.set(
        "clear",
        lua.create_function(move |_, (): ()| {
            let mut store = lock(&shared);
            store.entries().clear();
            persist(&store)
        })?,
    )?;

    Ok(table)
}

/// The whole store in memory, read from disk on first touch.
///
/// A `BTreeMap` so the file, and `keys()`, come out in a stable order: a store
/// that reshuffles itself on every write is noise in a diff and in a bug report.
struct Store {
    path: PathBuf,
    loaded: bool,
    values: BTreeMap<String, serde_json::Value>,
}

impl Store {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            loaded: false,
            values: BTreeMap::new(),
        }
    }

    fn entries(&mut self) -> &mut BTreeMap<String, serde_json::Value> {
        if !self.loaded {
            self.values = load(&self.path);
            self.loaded = true;
        }
        &mut self.values
    }

    fn flush(&self) -> Result<(), String> {
        let dir = self
            .path
            .parent()
            .ok_or_else(|| format!("{} has no parent directory", self.path.display()))?;
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;

        let body = serde_json::to_vec_pretty(&self.values)
            .map_err(|e| format!("cannot encode store: {e}"))?;

        // Write beside the target and rename: a rename within one directory
        // replaces the old file in one step, so an interrupted write leaves the
        // previous store intact instead of a truncated one. Losing a token to a
        // half-written file would look to the user like being randomly signed out.
        let temp = self
            .path
            .with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
        std::fs::write(&temp, &body)
            .map_err(|e| format!("cannot write {}: {e}", temp.display()))?;
        if let Err(e) = std::fs::rename(&temp, &self.path) {
            let _ = std::fs::remove_file(&temp);
            return Err(format!("cannot replace {}: {e}", self.path.display()));
        }
        Ok(())
    }
}

/// A corrupt or unreadable store starts empty rather than failing the plugin: the
/// plugin will find itself signed out and can run its wizard again, which is a far
/// better outcome than a source that refuses to load at all.
fn load(path: &Path) -> BTreeMap<String, serde_json::Value> {
    let body = match std::fs::read(path) {
        Ok(body) => body,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return BTreeMap::new(),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "cannot read plugin store");
            return BTreeMap::new();
        }
    };
    match serde_json::from_slice(&body) {
        Ok(values) => values,
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "plugin store is corrupt, starting empty"
            );
            BTreeMap::new()
        }
    }
}

/// A panic in one plugin call must not wedge that plugin's store for the rest of
/// the session, and the map is a plain value with no invariant a panic could have
/// broken halfway.
fn lock(state: &Arc<Mutex<Store>>) -> MutexGuard<'_, Store> {
    state.lock().unwrap_or_else(|e| e.into_inner())
}

/// A write that failed is reported as a plain runtime error, not a
/// [`fail`](super::fail) code: it is the host's disk misbehaving, not anything the
/// plugin got wrong or the user can act on.
fn persist(store: &Store) -> mlua::Result<()> {
    store.flush().map_err(|e| {
        tracing::warn!(error = %e, "cannot persist plugin store");
        mlua::Error::runtime(format!("cannot persist plugin store: {e}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kopuz-store-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    fn json(value: &str) -> serde_json::Value {
        serde_json::Value::String(value.to_string())
    }

    #[test]
    fn a_flushed_store_reloads() {
        let dir = temp_dir();
        let path = dir.join(FILE_NAME);

        let mut store = Store::new(path.clone());
        store.entries().insert("token".into(), json("abc"));
        store
            .entries()
            .insert("a/b".into(), json("slashes are keys"));
        store.flush().expect("flush");

        let mut reopened = Store::new(path.clone());
        assert_eq!(reopened.entries().get("token"), Some(&json("abc")));
        assert_eq!(
            reopened.entries().get("a/b"),
            Some(&json("slashes are keys"))
        );
        // A key with a '/' must not have become a directory.
        assert!(!dir.join("a").exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The temp file is the mechanism, not part of the result: it must be gone.
    #[test]
    fn the_write_leaves_nothing_behind() {
        let dir = temp_dir();
        let mut store = Store::new(dir.join(FILE_NAME));
        store.entries().insert("k".into(), json("v"));
        store.flush().expect("flush");
        store.flush().expect("flush again");

        let names: Vec<String> = std::fs::read_dir(&dir)
            .expect("read dir")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec![FILE_NAME.to_string()]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_corrupt_file_starts_empty_and_is_then_overwritten() {
        let dir = temp_dir();
        let path = dir.join(FILE_NAME);
        std::fs::write(&path, b"{ this is not json").expect("write");

        let mut store = Store::new(path.clone());
        assert!(store.entries().is_empty());

        store.entries().insert("token".into(), json("fresh"));
        store.flush().expect("flush");

        let mut reopened = Store::new(path);
        assert_eq!(reopened.entries().get("token"), Some(&json("fresh")));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_data_dir_is_created_on_first_write() {
        let dir = temp_dir();
        let nested = dir.join("plugin-data").join("example");
        let mut store = Store::new(nested.join(FILE_NAME));
        store.entries().insert("k".into(), json("v"));
        store.flush().expect("flush");
        assert!(nested.join(FILE_NAME).is_file());

        std::fs::remove_dir_all(&dir).ok();
    }
}
