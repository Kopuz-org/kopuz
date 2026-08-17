//! `kopuz.http`: the only way out to the network.
//!
//! Every function here is async and yields the Lua coroutine while it waits, so
//! a plugin writes straight-line code and never blocks a runtime thread:
//!
//! ```lua
//! local res = kopuz.http.post("https://example.test/token", {
//!   form = { grant_type = "refresh_token", refresh_token = token },
//!   timeout_ms = 10000,
//! })
//! if not res.ok then
//!   kopuz.fail("auth", "refresh failed with " .. res.status)
//! end
//! ```
//!
//! A non-2xx status is *not* an error: it comes back with `ok = false` for the
//! plugin to inspect, because backends signal an expired session with a 401 the
//! plugin needs to see. Only a transport failure raises, as
//! `kopuz:connectivity`. Timeouts count as transport failures.

use std::sync::Arc;
use std::time::Duration;

use mlua::{Lua, LuaSerdeExt, Table, Value};
use reqwest::header::{HeaderName, HeaderValue};

use super::HostCtx;

/// Request timeout when `opts.timeout_ms` is absent.
const DEFAULT_TIMEOUT_MS: u64 = 30_000;

/// Ceiling on `opts.timeout_ms`. The call deadline
/// ([`CALL_TIMEOUT`](crate::plugin::CALL_TIMEOUT)) fails the whole call anyway;
/// this only stops a plugin asking for a wait it can never be granted.
const MAX_TIMEOUT_MS: u64 = 120_000;

/// Redirect hops allowed when `follow_redirects` is on.
const MAX_REDIRECTS: usize = 10;

/// `request(opts)`, `get(url [,opts])`, `post(url [,opts])`, `head(url [,opts])`.
///
/// `opts` fields, all optional except a `url` from somewhere:
///
/// | field | meaning |
/// | --- | --- |
/// | `url` | the target. The positional argument of `get`/`post`/`head` wins over it. |
/// | `method` | defaults to `"GET"` on `request`, and to its own verb on the others. |
/// | `headers` | name to value. Names are lowercased. |
/// | `query` | appended to the URL, key-sorted so a signed query string is reproducible. Values may be a string, number or boolean. |
/// | `body` | a raw byte string, with no `Content-Type` of its own. |
/// | `json` | any value, encoded and sent as `application/json`. |
/// | `form` | a table, sent url-encoded as `application/x-www-form-urlencoded`. |
/// | `timeout_ms` | default 30000, capped at 120000. |
/// | `follow_redirects` | default true, up to 10 hops. |
///
/// At most one of `body`, `json` and `form` may be set. A `content-type` in
/// `headers` wins over the one `json` or `form` would have set for you.
///
/// The response table:
///
/// | field | meaning |
/// | --- | --- |
/// | `status` | the HTTP status as a number. |
/// | `ok` | true for 2xx. |
/// | `headers` | lowercased names to values. A name that repeated keeps its last value. |
/// | `body` | the raw bytes as a Lua string. |
/// | `url` | the final URL, after any redirects. |
/// | `json()` | the body decoded, raising `kopuz:invalid_input` if it is not JSON. |
pub(super) fn module(lua: &Lua, _ctx: &Arc<HostCtx>) -> mlua::Result<Table> {
    let clients = Arc::new(Clients::new());
    let table = lua.create_table()?;

    let shared = Arc::clone(&clients);
    let request = lua.create_async_function(move |lua, opts: Option<Table>| {
        let clients = Arc::clone(&shared);
        let parsed = Req::parse(&lua, None, opts, "GET");
        async move { response_table(&lua, send(&clients, parsed?).await?) }
    })?;
    table.set("request", request)?;

    for (name, verb) in [("get", "GET"), ("post", "POST"), ("head", "HEAD")] {
        let shared = Arc::clone(&clients);
        let f = lua.create_async_function(
            move |lua, (url, opts): (mlua::LuaString, Option<Table>)| {
                let clients = Arc::clone(&shared);
                let parsed = Req::parse(&lua, Some(url), opts, verb);
                async move { response_table(&lua, send(&clients, parsed?).await?) }
            },
        )?;
        table.set(name, f)?;
    }

    Ok(table)
}

/// One pair of clients per plugin, built once and shared by every closure.
///
/// Two of them because reqwest fixes its redirect policy on the client, not on
/// the request, and `follow_redirects = false` is how a plugin reads a `Location`
/// header (a common way for a backend to hand back a signed stream URL).
struct Clients {
    follow: reqwest::Client,
    manual: reqwest::Client,
}

impl Clients {
    fn new() -> Self {
        Self {
            follow: build(reqwest::redirect::Policy::limited(MAX_REDIRECTS)),
            manual: build(reqwest::redirect::Policy::none()),
        }
    }

    fn pick(&self, follow_redirects: bool) -> &reqwest::Client {
        if follow_redirects {
            &self.follow
        } else {
            &self.manual
        }
    }
}

fn build(policy: reqwest::redirect::Policy) -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("kopuz/", env!("CARGO_PKG_VERSION")))
        .redirect(policy)
        .build()
        .unwrap_or_default()
}

/// A request with every Lua value already copied out, so the future that sends it
/// borrows nothing from the interpreter.
struct Req {
    method: reqwest::Method,
    url: reqwest::Url,
    headers: Vec<(HeaderName, HeaderValue)>,
    query: Vec<(String, String)>,
    body: Option<Body>,
    timeout: Duration,
    follow_redirects: bool,
}

enum Body {
    Raw(Vec<u8>),
    Json(serde_json::Value),
    Form(Vec<(String, String)>),
}

impl Req {
    fn parse(
        lua: &Lua,
        url: Option<mlua::LuaString>,
        opts: Option<Table>,
        default_method: &str,
    ) -> mlua::Result<Self> {
        let opts = match opts {
            Some(opts) => opts,
            None => lua.create_table()?,
        };

        let from_opts = opts.get::<Option<mlua::LuaString>>("url")?;
        let Some(url) = url.or(from_opts) else {
            return Err(super::invalid_input("http: no url"));
        };
        let url = super::text_of(&url);
        let url = reqwest::Url::parse(&url)
            .map_err(|e| super::invalid_input(format!("http: cannot parse url {url:?}: {e}")))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(super::invalid_input(format!(
                "http: unsupported url scheme {:?}",
                url.scheme()
            )));
        }

        let method = match opts.get::<Option<mlua::LuaString>>("method")? {
            Some(m) => super::text_of(&m).to_ascii_uppercase(),
            None => default_method.to_string(),
        };
        let method = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|_| super::invalid_input(format!("http: invalid method {method:?}")))?;

        Ok(Self {
            method,
            url,
            headers: headers(&opts)?,
            query: pairs(&opts, "query")?,
            body: body(lua, &opts)?,
            timeout: timeout(&opts)?,
            follow_redirects: opts
                .get::<Option<bool>>("follow_redirects")?
                .unwrap_or(true),
        })
    }
}

fn headers(opts: &Table) -> mlua::Result<Vec<(HeaderName, HeaderValue)>> {
    let Some(table) = opts.get::<Option<Table>>("headers")? else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for entry in table.pairs::<Value, Value>() {
        let (key, value) = entry?;
        let key = super::scalar_text(&key, "http: a header name")?;
        let name = HeaderName::from_bytes(key.to_ascii_lowercase().as_bytes())
            .map_err(|_| super::invalid_input(format!("http: invalid header name {key:?}")))?;
        let text = super::scalar_text(&value, "http: a header value")?;
        let value = HeaderValue::from_bytes(text.as_bytes())
            .map_err(|_| super::invalid_input(format!("http: invalid value for header {key:?}")))?;
        out.push((name, value));
    }
    Ok(out)
}

/// A table of scalars as key-sorted pairs. Lua tables have no iteration order, so
/// sorting is what makes a query string (and a signature over it) reproducible.
fn pairs(opts: &Table, field: &str) -> mlua::Result<Vec<(String, String)>> {
    let Some(table) = opts.get::<Option<Table>>(field)? else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for entry in table.pairs::<Value, Value>() {
        let (key, value) = entry?;
        let key = super::scalar_text(&key, &format!("http: a {field} key"))?;
        let value = super::scalar_text(&value, &format!("http: the {field} value for {key:?}"))?;
        out.push((key, value));
    }
    out.sort();
    Ok(out)
}

fn body(lua: &Lua, opts: &Table) -> mlua::Result<Option<Body>> {
    let raw = opts.get::<Option<mlua::LuaString>>("body")?;
    let json = opts.get::<Value>("json")?;
    let form = opts.get::<Option<Table>>("form")?;

    let set =
        usize::from(raw.is_some()) + usize::from(!json.is_nil()) + usize::from(form.is_some());
    if set > 1 {
        return Err(super::invalid_input(
            "http: set at most one of body, json and form",
        ));
    }

    if let Some(raw) = raw {
        return Ok(Some(Body::Raw(super::bytes_of(&raw))));
    }
    if !json.is_nil() {
        let value: serde_json::Value = lua
            .from_value(json)
            .map_err(|e| super::invalid_input(format!("http: cannot encode json body: {e}")))?;
        return Ok(Some(Body::Json(value)));
    }
    if form.is_some() {
        return Ok(Some(Body::Form(pairs(opts, "form")?)));
    }
    Ok(None)
}

fn timeout(opts: &Table) -> mlua::Result<Duration> {
    let ms = match opts.get::<Option<f64>>("timeout_ms")? {
        None => DEFAULT_TIMEOUT_MS,
        Some(ms) if ms.is_finite() && ms >= 1.0 => (ms as u64).min(MAX_TIMEOUT_MS),
        Some(_) => {
            return Err(super::invalid_input(
                "http: timeout_ms must be a positive number of milliseconds",
            ));
        }
    };
    Ok(Duration::from_millis(ms))
}

/// A finished exchange, again with nothing borrowed from reqwest.
struct Res {
    status: u16,
    url: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

async fn send(clients: &Clients, req: Req) -> mlua::Result<Res> {
    let mut builder = clients
        .pick(req.follow_redirects)
        .request(req.method, req.url)
        .timeout(req.timeout);
    for (name, value) in req.headers {
        builder = builder.header(name, value);
    }
    if !req.query.is_empty() {
        builder = builder.query(&req.query);
    }
    builder = match req.body {
        Some(Body::Raw(bytes)) => builder.body(bytes),
        Some(Body::Json(value)) => builder.json(&value),
        Some(Body::Form(pairs)) => builder.form(&pairs),
        None => builder,
    };

    let response = builder
        .send()
        .await
        .map_err(|e| super::connectivity(describe(&e)))?;
    let status = response.status().as_u16();
    let url = response.url().to_string();
    let headers = response
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                // `HeaderName` is already lowercase; `http` normalises on parse.
                name.as_str().to_string(),
                String::from_utf8_lossy(value.as_bytes()).into_owned(),
            )
        })
        .collect();
    let body = response
        .bytes()
        .await
        .map_err(|e| super::connectivity(describe(&e)))?
        .to_vec();

    Ok(Res {
        status,
        url,
        headers,
        body,
    })
}

fn response_table(lua: &Lua, res: Res) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set("status", res.status)?;
    table.set("ok", (200..300).contains(&res.status))?;
    table.set("url", res.url)?;

    let headers = lua.create_table()?;
    for (name, value) in res.headers {
        headers.set(name, value)?;
    }
    table.set("headers", headers)?;

    table.set("body", lua.create_string(&res.body)?)?;

    let body = res.body;
    // Variadic so `res:json()` and `res.json()` both work.
    let json =
        lua.create_function(move |lua, _: mlua::MultiValue| super::json::decode(lua, &body))?;
    table.set("json", json)?;

    Ok(table)
}

/// reqwest hides the interesting part (DNS, TLS, connection refused) behind
/// `source`, and "error sending request" on its own tells a user nothing.
fn describe(err: &reqwest::Error) -> String {
    let mut out = err.to_string();
    let mut source = std::error::Error::source(err);
    for _ in 0..4 {
        let Some(cause) = source else { break };
        out.push_str(": ");
        out.push_str(&cause.to_string());
        source = cause.source();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_clamps_to_the_ceiling() {
        let lua = Lua::new();
        let opts = lua.create_table().expect("table");
        opts.set("timeout_ms", 999_999).expect("set");
        assert_eq!(
            timeout(&opts).expect("clamped"),
            Duration::from_millis(MAX_TIMEOUT_MS)
        );
    }

    #[test]
    fn timeout_defaults_when_absent() {
        let lua = Lua::new();
        let opts = lua.create_table().expect("table");
        assert_eq!(
            timeout(&opts).expect("default"),
            Duration::from_millis(DEFAULT_TIMEOUT_MS)
        );
    }

    #[test]
    fn a_non_positive_timeout_is_rejected() {
        let lua = Lua::new();
        let opts = lua.create_table().expect("table");
        opts.set("timeout_ms", 0).expect("set");
        assert!(timeout(&opts).is_err());
    }

    #[test]
    fn query_pairs_come_back_key_sorted() {
        let lua = Lua::new();
        let opts = lua.create_table().expect("table");
        let query = lua.create_table().expect("table");
        query.set("zulu", 1).expect("set");
        query.set("alpha", "a").expect("set");
        query.set("mike", true).expect("set");
        opts.set("query", query).expect("set");
        assert_eq!(
            pairs(&opts, "query").expect("pairs"),
            vec![
                ("alpha".to_string(), "a".to_string()),
                ("mike".to_string(), "true".to_string()),
                ("zulu".to_string(), "1".to_string()),
            ]
        );
    }

    #[test]
    fn two_bodies_are_rejected() {
        let lua = Lua::new();
        let opts = lua.create_table().expect("table");
        opts.set("body", "raw").expect("set");
        opts.set("json", 1).expect("set");
        assert!(body(&lua, &opts).is_err());
    }

    #[test]
    fn a_non_http_url_is_rejected() {
        let lua = Lua::new();
        let opts = lua.create_table().expect("table");
        opts.set("url", "file:///etc/passwd").expect("set");
        assert!(Req::parse(&lua, None, Some(opts), "GET").is_err());
    }

    #[test]
    fn header_names_are_lowercased() {
        let lua = Lua::new();
        let opts = lua.create_table().expect("table");
        let given = lua.create_table().expect("table");
        given.set("Authorization", "Bearer x").expect("set");
        opts.set("headers", given).expect("set");
        let parsed = headers(&opts).expect("headers");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].0.as_str(), "authorization");
    }
}
