//! In-app anonymous PoToken minter (issue #349).
//!
//! Stands up a hidden `wry` WebView at the music.youtube.com origin (same-origin
//! so BgUtils' WAA `fetch` isn't CORS-blocked), injects BgUtils, and mints
//! content pots on demand — feeding `server::ytmusic::botguard`'s channel. No
//! external binary, no yt-dlp. Linux/webkit2gtk only (declared `mod` is
//! cfg-gated in main.rs); other platforms fall back to no anon minting.
//!
//! Proven standalone in /tmp/yt-wry-minter. Tricks: same-origin page for the
//! WAA fetch, `trustedTypes.createPolicy` to `new Function` the BotGuard VM
//! past YouTube's CSP, and capturing `BG` at document-start before the page
//! clobbers `window.module`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use dioxus::desktop::tao::dpi::LogicalSize;
use dioxus::desktop::tao::event_loop::EventLoopWindowTarget;
use dioxus::desktop::tao::window::{Window, WindowBuilder};
use dioxus::desktop::wry::{WebView, WebViewBuilder};
use server::ytmusic::botguard::{self, MintRequest};
use tokio::sync::mpsc;

const BGUTILS: &str = include_str!("bgutils.js");
const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) \
                  Chrome/131.0.0.0 Safari/537.36";

static REQ_ID: AtomicU64 = AtomicU64::new(1);

type Pending = Rc<RefCell<HashMap<u64, tokio::sync::oneshot::Sender<Result<String, String>>>>>;

thread_local! {
    static INSTALLED: Cell<bool> = const { Cell::new(false) };
    // Keep the window + webview alive for the app's lifetime.
    static KEEPALIVE: RefCell<Option<(Window, Rc<WebView>)>> = const { RefCell::new(None) };
}

fn init_script() -> String {
    format!(
        r#"
window.module = {{ exports: {{}} }}; window.exports = window.module.exports;
{BGUTILS}
window.__KOPUZ_BG = (window.module && window.module.exports && window.module.exports.BG) || null;
window.__kopuzMint = async function(videoId, reqId) {{
  function send(o) {{ o.id = reqId; try {{ window.ipc.postMessage(JSON.stringify(o)); }} catch(e) {{}} }}
  try {{
    var BG = window.__KOPUZ_BG;
    if (!BG) return send({{err:'no BG'}});
    var bgConfig = {{
      fetch: function(u, o) {{ return fetch(u, o); }},
      globalObj: window, identifier: videoId, requestKey: "O43z0dpjhgX20SCx4KAo"
    }};
    var ch = await BG.Challenge.create(bgConfig);
    if (!ch) return send({{err:'null challenge'}});
    var js = ch.interpreterJavascript && ch.interpreterJavascript.privateDoNotAccessOrElseSafeScriptWrappedValue;
    if (js) {{
      var src = js;
      if (window.trustedTypes && window.trustedTypes.createPolicy) {{
        var pol = window.trustedTypes.createPolicy('kopuz-bg', {{ createScript: function(s){{ return s; }} }});
        src = pol.createScript(js);
      }}
      new Function(src)();
    }}
    var res = await BG.PoToken.generate({{ program: ch.program, globalName: ch.globalName, bgConfig: bgConfig }});
    send({{pot: (res && (res.poToken || res.pot || res)) + ''}});
  }} catch (e) {{ send({{err: (e && e.stack) ? e.stack : ('' + e)}}); }}
}};
"#
    )
}

/// Create the minter WebView once and register its channel with `botguard`.
/// Called from `Config::with_custom_event_handler` (gives the window target).
pub fn install<T: 'static>(target: &EventLoopWindowTarget<T>) {
    if INSTALLED.with(|c| c.replace(true)) {
        return;
    }

    // Tiny visible window so the native handle exists (wry needs a realized
    // window); hidden again right after the webview attaches.
    let window = match WindowBuilder::new()
        .with_title("kopuz pot minter")
        .with_inner_size(LogicalSize::new(1.0, 1.0))
        .with_visible(true)
        .build(target)
    {
        Ok(w) => w,
        Err(e) => {
            eprintln!("[pot-minter] window build failed: {e}");
            return;
        }
    };

    let pending: Pending = Rc::new(RefCell::new(HashMap::new()));
    let pending_ipc = pending.clone();

    let webview = WebViewBuilder::new()
        .with_url("https://music.youtube.com/")
        .with_user_agent(UA)
        .with_initialization_script(&init_script())
        .with_ipc_handler(move |req| {
            let body = req.into_body();
            let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            let Some(id) = v.get("id").and_then(|i| i.as_u64()) else {
                return;
            };
            let result = match v.get("pot").and_then(|p| p.as_str()) {
                Some(pot) => Ok(pot.to_string()),
                None => Err(v
                    .get("err")
                    .and_then(|e| e.as_str())
                    .unwrap_or("mint failed")
                    .to_string()),
            };
            if let Some(reply) = pending_ipc.borrow_mut().remove(&id) {
                let _ = reply.send(result);
            }
        })
        .build(&window);

    let webview = match webview {
        Ok(w) => Rc::new(w),
        Err(e) => {
            eprintln!("[pot-minter] webview build failed: {e}");
            return;
        }
    };
    let _ = window.set_visible(false);

    let (tx, mut rx) = mpsc::unbounded_channel::<MintRequest>();
    if botguard::set_minter(tx).is_err() {
        eprintln!("[pot-minter] minter already registered");
    }

    let wv = webview.clone();
    glib::MainContext::default().spawn_local(async move {
        while let Some(req) = rx.recv().await {
            let id = REQ_ID.fetch_add(1, Ordering::Relaxed);
            pending.borrow_mut().insert(id, req.reply);
            let vid: String = req.video_id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').collect();
            let _ = wv.evaluate_script(&format!("window.__kopuzMint && window.__kopuzMint('{vid}', {id})"));
        }
    });

    KEEPALIVE.with(|k| *k.borrow_mut() = Some((window, webview)));
    eprintln!("[pot-minter] installed (anon PoToken minting via webview)");
}
