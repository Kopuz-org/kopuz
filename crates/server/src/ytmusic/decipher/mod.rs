//! Native YouTube signature / `n` deciphering.
//!
//! YouTube's web clients (WEB_REMIX, the only client that exposes Premium
//! 256–270 kbps audio to a logged-in account) return stream formats wrapped
//! in a `signatureCipher`: the playable `url` is missing its `sig` query
//! param, and the `n` throttle param has to be transformed. Both transforms
//! are arbitrary obfuscated JS living inside YouTube's player `base.js`
//! (~2.5 MB, rotated every few hours). The robust way to apply them is to
//! *run YouTube's own JS* rather than re-implement it (which breaks on every
//! rotation). This module does exactly that, turning a `signatureCipher`
//! format into a playable URL — the thing that lets us serve Premium streams
//! without shelling out to yt-dlp. See issue #349.
//!
//! ## Engine seam
//!
//! Actual JS execution is abstracted behind [`JsEngine`]. The end goal is to
//! run it inside Dioxus' own resident WebView JavaScriptCore — zero extra
//! dependencies, since the engine is already loaded to render the UI. That
//! binding lives in the UI layer (it needs `dioxus::document::eval`) and is
//! injected here via [`set_engine`]. Until one is registered we fall back to
//! a system JS runtime ([`SubprocessEngine`]: deno / node / bun / qjs), so
//! the path works headlessly, in tests, and on first run before the WebView
//! engine is wired. If no runtime is available either, deciphering fails and
//! the caller falls through to its existing chain — strictly additive.
//!
//! ## Solver scripts
//!
//! `solver/lib.min.js` + `solver/core.min.js` are vendored verbatim from
//! yt-dlp's `yt_dlp_ejs` package (Unlicense / public domain). They bundle a
//! JS parser (meriyah + astring, ISC / MIT) and the `jsc()` challenge
//! orchestrator. We do not modify them — refresh from upstream when
//! YouTube's player format changes. See `solver/NOTICE` for attribution.

use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};
use tokio::sync::{OnceCell, mpsc, oneshot};

const LIB: &str = include_str!("solver/lib.min.js");
const CORE: &str = include_str!("solver/core.min.js");
const WEB_UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:140.0) Gecko/20100101 Firefox/140.0";

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A backend that can execute a self-contained JS program and return what it
/// printed to stdout. The program (built by [`solve`]) loads the vendored
/// solver, runs `jsc(...)`, and prints exactly one line: the JSON-encoded
/// `responses` array.
pub trait JsEngine: Send + Sync {
    fn run<'a>(&'a self, program: String) -> BoxFuture<'a, Result<String, String>>;
}

static ENGINE: OnceLock<Box<dyn JsEngine>> = OnceLock::new();

/// Register the JS engine to use for deciphering. Intended for the UI layer
/// to install a WebView-backed engine at startup. No-op (returns `Err`) if an
/// engine was already resolved — register before the first track resolves.
pub fn set_engine(engine: Box<dyn JsEngine>) -> Result<(), Box<dyn JsEngine>> {
    ENGINE.set(engine)
}

fn engine() -> &'static dyn JsEngine {
    ENGINE.get_or_init(|| Box::new(SubprocessEngine)).as_ref()
}

/// Build the playable URL for one `adaptiveFormats[]` entry. Handles both
/// `signatureCipher` formats (solve `sig` + `n`) and plain `url` formats that
/// still carry an `n` throttle param (solve `n` only). Returns the format's
/// `url` untouched when there's nothing to solve.
pub async fn deciphered_url(base_js: &str, format: &Value) -> Result<String, String> {
    let (mut url, sig, sp) = extract_cipher(format)?;
    let n = query_param(&url, "n");

    let mut requests = Vec::new();
    if let Some(n) = &n {
        requests.push(json!({ "type": "n", "challenges": [n] }));
    }
    if let Some(s) = &sig {
        requests.push(json!({ "type": "sig", "challenges": [s] }));
    }
    if requests.is_empty() {
        return Ok(url); // plain url, no n — nothing to do
    }

    let responses = solve(base_js, &requests).await?;

    if let (Some(old), Some(new)) = (&n, n.as_ref().and_then(|n| lookup(&responses, n))) {
        url = replace_query_value(&url, "n", old, &new);
    }
    if let Some(s) = &sig {
        let solved = lookup(&responses, s).ok_or("signature solve produced no result")?;
        url.push_str(&format!("&{sp}={}", pct_encode(&solved)));
    }
    Ok(url)
}

/// Run the solver over `requests` against `base_js`, returning the parsed
/// `responses` array.
async fn solve(base_js: &str, requests: &[Value]) -> Result<Value, String> {
    let data = json!({ "type": "player", "player": base_js, "requests": requests });
    let data_json = serde_json::to_string(&data).map_err(|e| format!("encode solver data: {e}"))?;
    // yt_dlp_ejs sets `globalThis.location = new URL(".../watch?v=yt-dlp-wins")`
    // as environment setup. Harmless in node/deno, but in a real WebView
    // globalThis === window, so it NAVIGATES (out to the browser). Rename the
    // write to a dummy property — the extraction passes the URL explicitly and
    // doesn't read window.location (verified: decipher works without it).
    let core = CORE.replace("globalThis.location =", "globalThis.__kopuz_loc =");
    // `print` (JSC/qjs) or `console.log` (node/deno/bun) — whichever exists.
    let program = format!(
        "{LIB}\nObject.assign(globalThis, lib);\n{core}\n\
         (function(){{var __p=(typeof print==='function')?print:function(s){{console.log(s);}};\
         var o=jsc({data_json});__p(JSON.stringify(o.responses));}})();"
    );
    let stdout = engine().run(program).await?;
    let line = stdout
        .lines()
        .rev()
        .find(|l| l.trim_start().starts_with('['))
        .unwrap_or_else(|| stdout.trim());
    serde_json::from_str(line).map_err(|e| {
        let head: String = stdout.chars().take(160).collect();
        format!("solver output parse ({e}); got: {head}")
    })
}

/// Pull a solved value out of the `responses` array by its input key. Each
/// response is `{type:"result", data:{ "<input>": "<output>" }}`.
fn lookup(responses: &Value, key: &str) -> Option<String> {
    responses.as_array()?.iter().find_map(|r| {
        r.pointer("/data")?
            .as_object()?
            .get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    })
}

/// Split a format into `(url, signature, sig_param_name)`. `signature` is
/// `Some` only for `signatureCipher` formats.
fn extract_cipher(format: &Value) -> Result<(String, Option<String>, String), String> {
    if let Some(sc) = format.get("signatureCipher").and_then(|v| v.as_str()) {
        let (mut s, mut url, mut sp) = (String::new(), String::new(), String::from("sig"));
        for kv in sc.split('&') {
            if let Some((k, v)) = kv.split_once('=') {
                let v = pct_decode(v);
                match k {
                    "s" => s = v,
                    "url" => url = v,
                    "sp" => sp = v,
                    _ => {}
                }
            }
        }
        if url.is_empty() {
            return Err("signatureCipher missing url".into());
        }
        Ok((url, Some(s), sp))
    } else if let Some(u) = format.get("url").and_then(|v| v.as_str()) {
        Ok((u.to_string(), None, "sig".into()))
    } else {
        Err("format has neither signatureCipher nor url".into())
    }
}

fn query_param(url: &str, key: &str) -> Option<String> {
    url.split(['?', '&']).find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then(|| pct_decode(v))
    })
}

/// Replace the first `key=old` occurrence with `key=new` (values raw/undecoded
/// in the URL). `n` and the solved `n` are URL-safe base64, so no re-encoding.
fn replace_query_value(url: &str, key: &str, old: &str, new: &str) -> String {
    url.replacen(&format!("{key}={old}"), &format!("{key}={new}"), 1)
}

fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                Ok(h) => {
                    out.push(h);
                    i += 3;
                }
                Err(_) => {
                    out.push(b'%');
                    i += 1;
                }
            },
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn pct_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ---- base.js (player JS) fetch + signatureTimestamp cache --------------

/// Cached `(base_js, signature_timestamp)`. `base.js` rotates every few
/// hours; the matching `signatureTimestamp` must be sent on `/player` so the
/// `signatureCipher` YouTube returns corresponds to the `base.js` we hold.
/// Caching the pair keeps them consistent. Refreshed on process restart (and
/// callers should re-fetch if decipher starts failing after a rotation).
static PLAYER_JS: OnceCell<(String, u64)> = OnceCell::const_new();

/// Fetch (once, then cached) YouTube's player `base.js` and its embedded
/// `signatureTimestamp`. Seeded from any `video_id`'s watch page.
pub async fn player_js(video_id: &str) -> Result<&'static (String, u64), String> {
    PLAYER_JS
        .get_or_try_init(|| async { fetch_player_js(video_id).await })
        .await
}

async fn fetch_player_js(video_id: &str) -> Result<(String, u64), String> {
    let http = super::innertube::http_client();
    let watch = http
        .get(format!(
            "https://www.youtube.com/watch?v={video_id}&bpctr=9999999999"
        ))
        .header("User-Agent", WEB_UA)
        .send()
        .await
        .map_err(|e| format!("watch page fetch: {e}"))?
        .text()
        .await
        .map_err(|e| format!("watch page body: {e}"))?;
    let raw = str_between(&watch, "\"jsUrl\":\"", "\"")
        .ok_or("no jsUrl in watch page")?
        .replace("\\/", "/");
    let js_url = if raw.starts_with("http") {
        raw
    } else {
        format!("https://www.youtube.com{raw}")
    };
    let base_js = http
        .get(&js_url)
        .header("User-Agent", WEB_UA)
        .send()
        .await
        .map_err(|e| format!("base.js fetch: {e}"))?
        .text()
        .await
        .map_err(|e| format!("base.js body: {e}"))?;
    let sts = base_js
        .split("signatureTimestamp:")
        .nth(1)
        .and_then(|s| {
            s.split(|c: char| !c.is_ascii_digit())
                .find(|x| !x.is_empty())
        })
        .and_then(|s| s.parse::<u64>().ok())
        .ok_or("no signatureTimestamp in base.js")?;
    Ok((base_js, sts))
}

fn str_between<'a>(haystack: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let i = haystack.find(start)? + start.len();
    let rest = &haystack[i..];
    let j = rest.find(end)?;
    Some(&rest[..j])
}

// ---- default engine: shell out to a system JS runtime ------------------

#[derive(Clone, Copy)]
struct Runtime {
    bin: &'static str,
    args: &'static [&'static str],
}

fn detect_runtime() -> Option<Runtime> {
    static RT: OnceLock<Option<Runtime>> = OnceLock::new();
    *RT.get_or_init(|| {
        const CANDIDATES: &[Runtime] = &[
            Runtime { bin: "deno", args: &["run", "--quiet", "--no-prompt"] },
            Runtime { bin: "node", args: &[] },
            Runtime { bin: "bun", args: &["run"] },
            Runtime { bin: "qjs", args: &[] },
        ];
        CANDIDATES.iter().copied().find(|c| {
            std::process::Command::new(c.bin)
                .arg("--version")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        })
    })
}

/// Runs the solver in a one-shot system JS runtime subprocess. The fallback
/// when no WebView engine is registered. Carries the same correctness as the
/// WebView path (it's the same V8/JSC-class engine running the same script);
/// it just pays a process spawn instead of reusing the resident isolate.
pub struct SubprocessEngine;

impl JsEngine for SubprocessEngine {
    fn run<'a>(&'a self, program: String) -> BoxFuture<'a, Result<String, String>> {
        Box::pin(async move {
            let rt = detect_runtime().ok_or(
                "no JS runtime (deno/node/bun/qjs) found for native decipher; \
                 install one or register a WebView engine",
            )?;
            static SEQ: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "kopuz-yt-solve-{}-{}.js",
                std::process::id(),
                SEQ.fetch_add(1, Ordering::Relaxed)
            ));
            tokio::fs::write(&path, program)
                .await
                .map_err(|e| format!("write solver temp: {e}"))?;
            let out = tokio::process::Command::new(rt.bin)
                .args(rt.args)
                .arg(&path)
                .output()
                .await
                .map_err(|e| format!("spawn {}: {e}", rt.bin));
            let _ = tokio::fs::remove_file(&path).await;
            let out = out?;
            if !out.status.success() {
                let err = String::from_utf8_lossy(&out.stderr);
                let head: String = err.chars().take(200).collect();
                return Err(format!("{} exit {}: {head}", rt.bin, out.status));
            }
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
        })
    }
}

// ---- WebView engine bridge ---------------------------------------------

/// A unit of work for the UI-layer solver loop: a JS `program` to run in the
/// resident WebView, plus a one-shot `reply` for whatever it prints.
pub struct SolveRequest {
    pub program: String,
    pub reply: oneshot::Sender<Result<String, String>>,
}

/// [`JsEngine`] that forwards each program to a solver loop running in the UI
/// layer, which executes it via `dioxus::document::eval` inside the WebView's
/// own JavaScriptCore — the zero-external-dependency path (issue #349). Built
/// by [`webview_channel`]; the UI registers it via [`set_engine`] and drains
/// the returned receiver.
pub struct ChannelEngine {
    tx: mpsc::UnboundedSender<SolveRequest>,
}

impl JsEngine for ChannelEngine {
    fn run<'a>(&'a self, program: String) -> BoxFuture<'a, Result<String, String>> {
        Box::pin(async move {
            let (reply, rx) = oneshot::channel();
            self.tx
                .send(SolveRequest { program, reply })
                .map_err(|_| "webview solver loop is gone".to_string())?;
            rx.await
                .map_err(|_| "webview solver dropped the reply".to_string())?
        })
    }
}

/// Create a WebView-backed engine plus the receiver its solver loop drains.
/// The UI calls this once at startup, `set_engine`s the returned engine, and
/// spawns a Dioxus task that runs each `SolveRequest.program` via
/// `document::eval` and answers on `reply`.
pub fn webview_channel() -> (Box<dyn JsEngine>, mpsc::UnboundedReceiver<SolveRequest>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (Box::new(ChannelEngine { tx }), rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pct_roundtrip() {
        assert_eq!(pct_decode("a%3Db%2Fc"), "a=b/c");
        assert_eq!(pct_decode("hello%20world"), "hello world");
        assert_eq!(pct_encode("a=b/c+d"), "a%3Db%2Fc%2Bd");
        // url-safe chars pass through untouched
        assert_eq!(pct_encode("Ab9-_.~"), "Ab9-_.~");
    }

    #[test]
    fn extract_signature_cipher() {
        let f = json!({
            "signatureCipher": "s=SCRAMBLED&sp=sig&url=https%3A%2F%2Fr1.googlevideo.com%2Fvideoplayback%3Fn%3DTOKEN"
        });
        let (url, sig, sp) = extract_cipher(&f).unwrap();
        assert_eq!(sig.as_deref(), Some("SCRAMBLED"));
        assert_eq!(sp, "sig");
        assert_eq!(url, "https://r1.googlevideo.com/videoplayback?n=TOKEN");
        assert_eq!(query_param(&url, "n").as_deref(), Some("TOKEN"));
    }

    #[test]
    fn extract_plain_url() {
        let f = json!({ "url": "https://r1.googlevideo.com/videoplayback?n=TOKEN" });
        let (url, sig, _) = extract_cipher(&f).unwrap();
        assert!(sig.is_none());
        assert_eq!(query_param(&url, "n").as_deref(), Some("TOKEN"));
    }

    #[test]
    fn replace_n_and_lookup() {
        let url = "https://r1.googlevideo.com/videoplayback?n=OLD&mime=audio";
        assert_eq!(
            replace_query_value(url, "n", "OLD", "NEW"),
            "https://r1.googlevideo.com/videoplayback?n=NEW&mime=audio"
        );
        let responses = json!([
            { "type": "result", "data": { "OLD": "NEW" } },
            { "type": "result", "data": { "SCRAMBLED": "UNSCRAMBLED" } }
        ]);
        assert_eq!(lookup(&responses, "OLD").as_deref(), Some("NEW"));
        assert_eq!(lookup(&responses, "SCRAMBLED").as_deref(), Some("UNSCRAMBLED"));
        assert_eq!(lookup(&responses, "MISSING"), None);
    }

    #[test]
    fn str_between_basic() {
        assert_eq!(
            str_between(r#"x"jsUrl":"/s/player/abc/base.js"y"#, "\"jsUrl\":\"", "\""),
            Some("/s/player/abc/base.js")
        );
    }

    /// End-to-end against live YouTube via the SubprocessEngine: fetch base.js,
    /// do an anonymous WEB_REMIX /player (signatureCipher formats), decipher
    /// the best one, and confirm the URL streams (HTTP 206). Anonymous tops
    /// out at ~128 kbps (itag 251); the Premium path is identical but cookied.
    #[tokio::test]
    #[ignore = "hits live YouTube + needs a system JS runtime; run manually"]
    async fn live_anon_decipher_streams() {
        use super::super::{clients::WEB_REMIX, innertube};
        let vid = "dQw4w9WgXcQ";
        let player = player_js(vid).await.expect("base.js");
        let extras = innertube::PlayerExtras {
            signature_timestamp: Some(player.1),
            ..Default::default()
        };
        let json = innertube::player(WEB_REMIX, vid, None, extras)
            .await
            .expect("player");
        let fmt = json
            .pointer("/streamingData/adaptiveFormats")
            .and_then(|v| v.as_array())
            .expect("formats")
            .iter()
            .filter(|f| {
                f["mimeType"]
                    .as_str()
                    .unwrap_or("")
                    .starts_with("audio/")
            })
            .max_by_key(|f| f["bitrate"].as_u64().unwrap_or(0))
            .expect("audio format");
        assert!(
            fmt.get("signatureCipher").is_some(),
            "WEB_REMIX should return signed formats"
        );
        let url = deciphered_url(&player.0, fmt).await.expect("decipher");
        let resp = reqwest::Client::new()
            .get(&url)
            .header("Range", "bytes=0-1023")
            .send()
            .await
            .expect("range GET");
        assert_eq!(resp.status().as_u16(), 206, "deciphered URL must stream");
    }
}
