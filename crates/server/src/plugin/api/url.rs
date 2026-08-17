//! `kopuz.url`: percent-encoding and query strings.
//!
//! Encoding is RFC 3986 for a *query component*: everything outside
//! `A-Za-z0-9-_.~` is escaped, so a space becomes `%20` and never `+`. Decoding is
//! the exact inverse and leaves `+` alone. That pairing is what makes
//! [`build_query`] safe to sign: several backends (Subsonic-likes and the older
//! OAuth 1 flows among them) hash the query string they expect you to send, and a
//! `+` that means "space" on one side and "plus" on the other silently breaks
//! every signature.
//!
//! Keys come back sorted for the same reason: a Lua table has no iteration order,
//! so without sorting the same parameters would produce a different string, and a
//! different signature, on every call.

use std::sync::Arc;

use mlua::{Lua, Table, Value};

use super::HostCtx;

/// `encode`, `decode`, `build_query`, `parse_query` and `join`.
///
/// | function | result |
/// | --- | --- |
/// | `encode(s)` | `s` percent-encoded for a query component |
/// | `decode(s)` | the bytes back, `%xx` resolved |
/// | `build_query(t)` | `"a=1&b=2"`, keys sorted, keys and values encoded |
/// | `parse_query(s)` | a table of decoded pairs, an optional leading `?` allowed |
/// | `join(base, relative)` | `relative` resolved against `base` |
///
/// `join` follows RFC 3986, so the last path segment of `base` is replaced unless
/// `base` ends in `/`:
///
/// ```lua
/// kopuz.url.join("https://a.test/api/v1/", "tracks")  --> https://a.test/api/v1/tracks
/// kopuz.url.join("https://a.test/api/v1", "tracks")   --> https://a.test/api/tracks
/// kopuz.url.join("https://a.test/api/v1", "/tracks")  --> https://a.test/tracks
/// ```
pub(super) fn module(lua: &Lua, _ctx: &Arc<HostCtx>) -> mlua::Result<Table> {
    let table = lua.create_table()?;

    table.set(
        "encode",
        lua.create_function(|_, s: mlua::LuaString| {
            Ok(urlencoding::encode_binary(&super::bytes_of(&s)).into_owned())
        })?,
    )?;
    table.set(
        "decode",
        lua.create_function(|lua, s: mlua::LuaString| {
            let decoded = urlencoding::decode_binary(&super::bytes_of(&s)).into_owned();
            lua.create_string(decoded)
        })?,
    )?;

    table.set(
        "build_query",
        lua.create_function(|_, t: Table| Ok(build_query(collect(&t)?)))?,
    )?;
    table.set(
        "parse_query",
        lua.create_function(|lua, s: mlua::LuaString| {
            let out = lua.create_table()?;
            // A repeated key keeps its last value: this is a table, not a
            // multimap. Parse by hand if a backend really sends `a=1&a=2`.
            for (key, value) in parse_query(&super::text_of(&s)) {
                out.set(key, value)?;
            }
            Ok(out)
        })?,
    )?;

    table.set(
        "join",
        lua.create_function(|_, (base, relative): (mlua::LuaString, mlua::LuaString)| {
            join(&super::text_of(&base), &super::text_of(&relative))
        })?,
    )?;

    Ok(table)
}

/// A table of scalars as pairs, for [`build_query`].
fn collect(table: &Table) -> mlua::Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for entry in table.pairs::<Value, Value>() {
        let (key, value) = entry?;
        let key = super::scalar_text(&key, "a query key")?;
        let value = super::scalar_text(&value, &format!("the query value for {key:?}"))?;
        out.push((key, value));
    }
    Ok(out)
}

fn build_query(mut pairs: Vec<(String, String)>) -> String {
    pairs.sort();
    pairs
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                urlencoding::encode(key),
                urlencoding::encode(value)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

fn parse_query(query: &str) -> Vec<(String, String)> {
    let query = query.strip_prefix('?').unwrap_or(query);
    query
        .split('&')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let (key, value) = part.split_once('=').unwrap_or((part, ""));
            (decode_lossy(key), decode_lossy(value))
        })
        .collect()
}

fn decode_lossy(s: &str) -> String {
    String::from_utf8_lossy(&urlencoding::decode_binary(s.as_bytes())).into_owned()
}

fn join(base: &str, relative: &str) -> mlua::Result<String> {
    let parsed = reqwest::Url::parse(base)
        .map_err(|e| super::invalid_input(format!("url.join: bad base {base:?}: {e}")))?;
    let joined = parsed
        .join(relative)
        .map_err(|e| super::invalid_input(format!("url.join: cannot join {relative:?}: {e}")))?;
    Ok(joined.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(input: &[(&str, &str)]) -> Vec<(String, String)> {
        input
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn build_query_sorts_by_key() {
        let query = build_query(pairs(&[("zulu", "1"), ("alpha", "2"), ("mike", "3")]));
        assert_eq!(query, "alpha=2&mike=3&zulu=1");
    }

    #[test]
    fn build_query_encodes_both_sides_and_never_emits_plus() {
        let query = build_query(pairs(&[("q", "miles davis & co"), ("path", "a/b")]));
        assert_eq!(query, "path=a%2Fb&q=miles%20davis%20%26%20co");
    }

    #[test]
    fn build_query_of_nothing_is_empty() {
        assert_eq!(build_query(Vec::new()), "");
    }

    #[test]
    fn parse_query_decodes_and_tolerates_a_leading_question_mark() {
        assert_eq!(
            parse_query("?q=miles%20davis&limit=50"),
            pairs(&[("q", "miles davis"), ("limit", "50")])
        );
    }

    #[test]
    fn parse_query_handles_valueless_and_empty_parts() {
        assert_eq!(
            parse_query("flag&a=&&b=2"),
            pairs(&[("flag", ""), ("a", ""), ("b", "2")])
        );
        assert!(parse_query("").is_empty());
    }

    /// `+` stays a literal plus so that encode and decode are strict inverses.
    #[test]
    fn parse_query_leaves_plus_alone() {
        assert_eq!(parse_query("q=a+b"), pairs(&[("q", "a+b")]));
    }

    #[test]
    fn encode_and_decode_round_trip_binary() {
        let bytes: Vec<u8> = (0..=u8::MAX).collect();
        let encoded = urlencoding::encode_binary(&bytes).into_owned();
        assert_eq!(
            urlencoding::decode_binary(encoded.as_bytes()).into_owned(),
            bytes
        );
    }

    #[test]
    fn join_follows_rfc_3986() {
        assert_eq!(
            join("https://a.test/api/v1/", "tracks").expect("join"),
            "https://a.test/api/v1/tracks"
        );
        assert_eq!(
            join("https://a.test/api/v1", "tracks").expect("join"),
            "https://a.test/api/tracks"
        );
        assert_eq!(
            join("https://a.test/api/v1", "/tracks").expect("join"),
            "https://a.test/tracks"
        );
        assert_eq!(
            join("https://a.test/api/", "https://b.test/x").expect("join"),
            "https://b.test/x"
        );
    }

    #[test]
    fn join_rejects_a_base_that_is_not_absolute() {
        assert!(join("/api/v1", "tracks").is_err());
    }
}
