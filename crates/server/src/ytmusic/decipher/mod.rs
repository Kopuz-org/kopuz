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
//! Actual JS execution is abstracted behind [`JsEngine`]. On desktop the
//! default is [`DenoCoreEngine`], an in-process `deno_core` V8 isolate — no
//! WebView (the Blitz renderer removes it) and no external runtime. Android
//! (no V8) falls back to a system JS runtime ([`SubprocessEngine`]: deno /
//! node / bun / qjs). A different engine can be injected via [`set_engine`]
//! before the first solve. If nothing can run the solver, deciphering fails
//! and the caller falls through to its existing chain — strictly additive.
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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

// Headless deno_core decipher engine (the post-WebView path). Not on Android,
// which keeps the WebView.
#[cfg(not(target_os = "android"))]
mod deno_engine;
#[cfg(not(target_os = "android"))]
pub use deno_engine::DenoCoreEngine;

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
    ENGINE
        .get_or_init(|| {
            // Desktop: the headless deno_core isolate (no WebView, post-Blitz).
            // Android has no V8, so it keeps the WebView engine (installed via
            // `set_engine` at startup) with the system-runtime subprocess as the
            // fallback if that isn't wired.
            #[cfg(not(target_os = "android"))]
            {
                Box::new(DenoCoreEngine::new()) as Box<dyn JsEngine>
            }
            #[cfg(target_os = "android")]
            {
                Box::new(SubprocessEngine) as Box<dyn JsEngine>
            }
        })
        .as_ref()
}

/// Build the playable URL for one `adaptiveFormats[]` entry. Handles both
/// `signatureCipher` formats (solve `sig` + `n`) and plain `url` formats that
/// still carry an `n` throttle param (solve `n` only). Returns the format's
/// `url` untouched when there's nothing to solve.
#[tracing::instrument(name = "yt.decipher", skip(base_js, format))]
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

/// Cached `(base_js, signature_timestamp)` with a refresh TTL. `base.js` rotates
/// every few hours and its `signatureTimestamp` must match, so we re-fetch the
/// pair periodically instead of pinning it for the whole process — otherwise a
/// rotation would break decipher until restart.
type CachedPlayerJs = Arc<(String, u64)>;
type PlayerJsCacheEntry = (Instant, CachedPlayerJs);

static PLAYER_JS: Mutex<Option<PlayerJsCacheEntry>> = Mutex::new(None);
const PLAYER_JS_TTL: Duration = Duration::from_secs(60 * 60);

/// Fetch YouTube's player `base.js` and its embedded `signatureTimestamp`,
/// cached for [`PLAYER_JS_TTL`]. Seeded from any `video_id`'s watch page.
#[tracing::instrument(name = "yt.player_js", fields(video_id = %video_id))]
pub async fn player_js(video_id: &str) -> Result<CachedPlayerJs, String> {
    if let Ok(g) = PLAYER_JS.lock()
        && let Some((at, data)) = g.as_ref()
        && at.elapsed() < PLAYER_JS_TTL
    {
        return Ok(data.clone());
    }
    let data = Arc::new(fetch_player_js(video_id).await?);
    if let Ok(mut g) = PLAYER_JS.lock() {
        *g = Some((Instant::now(), data.clone()));
    }
    Ok(data)
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
            Runtime {
                bin: "deno",
                args: &["run", "--quiet", "--no-prompt"],
            },
            Runtime {
                bin: "node",
                args: &[],
            },
            Runtime {
                bin: "bun",
                args: &["run"],
            },
            Runtime {
                bin: "qjs",
                args: &[],
            },
        ];
        CANDIDATES.iter().copied().find(|c| {
            let mut cmd = std::process::Command::new(c.bin);
            cmd.arg("--version")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            // Don't flash a console window on Windows for the version probe.
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
            }
            cmd.status().map(|s| s.success()).unwrap_or(false)
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
            // O_EXCL: never write through a pre-existing (possibly symlinked) path.
            {
                use tokio::io::AsyncWriteExt;
                let mut f = tokio::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .await
                    .map_err(|e| format!("create solver temp: {e}"))?;
                f.write_all(program.as_bytes())
                    .await
                    .map_err(|e| format!("write solver temp: {e}"))?;
            }
            let mut cmd = tokio::process::Command::new(rt.bin);
            cmd.args(rt.args)
                .arg(&path)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true);
            // Don't flash a console window on Windows for the solver subprocess.
            #[cfg(target_os = "windows")]
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
            let child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => {
                    let _ = tokio::fs::remove_file(&path).await;
                    return Err(format!("spawn {}: {e}", rt.bin));
                }
            };
            // Bound it — a hung runtime must not stall deciphering. On timeout the
            // dropped future kills the child (kill_on_drop).
            let out =
                tokio::time::timeout(std::time::Duration::from_secs(30), child.wait_with_output())
                    .await;
            let _ = tokio::fs::remove_file(&path).await;
            let out = match out {
                Ok(Ok(o)) => o,
                Ok(Err(e)) => return Err(format!("{}: {e}", rt.bin)),
                Err(_) => return Err(format!("{} solver timed out (30s)", rt.bin)),
            };
            if !out.status.success() {
                let err = String::from_utf8_lossy(&out.stderr);
                let head: String = err.chars().take(200).collect();
                return Err(format!("{} exit {}: {head}", rt.bin, out.status));
            }
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
        })
    }
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
        assert_eq!(
            lookup(&responses, "SCRAMBLED").as_deref(),
            Some("UNSCRAMBLED")
        );
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
            .filter(|f| f["mimeType"].as_str().unwrap_or("").starts_with("audio/"))
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

    /// Same end-to-end proof, but through the headless [`DenoCoreEngine`] (no
    /// WebView, no system runtime) — the post-Blitz decipher path. Run alone:
    /// `cargo test -p kopuz-server deno_decipher -- --ignored --nocapture`.
    #[cfg(not(target_os = "android"))]
    #[tokio::test]
    #[ignore = "hits live YouTube; run manually and alone (registers the global engine)"]
    async fn live_deno_decipher_streams() {
        use super::super::{clients::WEB_REMIX, innertube};
        super::set_engine(Box::new(super::DenoCoreEngine::new()))
            .unwrap_or_else(|_| panic!("engine already set — run this test alone"));
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
            .filter(|f| f["mimeType"].as_str().unwrap_or("").starts_with("audio/"))
            .max_by_key(|f| f["bitrate"].as_u64().unwrap_or(0))
            .expect("audio format");
        let t = std::time::Instant::now();
        let url = deciphered_url(&player.0, fmt).await.expect("decipher");
        eprintln!("deno cold decipher: {} ms", t.elapsed().as_millis());
        // A second solve on the warm isolate, to eyeball the warm cost.
        let t2 = std::time::Instant::now();
        let _ = deciphered_url(&player.0, fmt).await.expect("decipher 2");
        eprintln!("deno warm decipher: {} ms", t2.elapsed().as_millis());
        let resp = reqwest::Client::new()
            .get(&url)
            .header("Range", "bytes=0-1023")
            .send()
            .await
            .expect("range GET");
        assert_eq!(
            resp.status().as_u16(),
            206,
            "deno-deciphered URL must stream"
        );
    }
}
