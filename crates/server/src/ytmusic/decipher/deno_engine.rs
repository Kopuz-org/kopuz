//! Headless [`JsEngine`] backed by an in-process `deno_core` V8 isolate — the
//! decipher path once the WebView (and its JavaScriptCore) is gone. sig/n are
//! pure computation, so a bare isolate runs the vendored `yt_dlp_ejs` solver
//! directly. Lives on one dedicated thread (`!Send`), reused across tracks to
//! amortize V8 init and warm the JIT.

use std::cell::RefCell;

use deno_core::{JsRuntime, PollEventLoopOptions, RuntimeOptions, op2};
use tokio::sync::{mpsc, oneshot};

use super::JsEngine;

// Per-run capture of the solver's `print` output (thread-local: the isolate is
// single-threaded).
thread_local! {
    static OUT: RefCell<String> = const { RefCell::new(String::new()) };
}

#[op2(fast)]
fn op_capture(#[string] s: String) {
    OUT.with(|o| {
        let mut b = o.borrow_mut();
        b.push_str(&s);
        b.push('\n');
    });
}

deno_core::extension!(decipher_ext, ops = [op_capture]);

/// Browser globals the solver expects but bare `deno_core` lacks: `print`/
/// `console.log` → capture op, plus a minimal `URL`/`location`.
const PRELUDE: &str = r#"
globalThis.print = function(s) { Deno.core.ops.op_capture(String(s)); };
globalThis.console = {
  log: globalThis.print, info: globalThis.print,
  warn: globalThis.print, error: globalThis.print, debug: function() {}
};
// base.js reads .hostname/.origin off URLs and `location`; a light parse covers it.
if (typeof globalThis.URL !== 'function') {
  globalThis.URL = function(u) {
    u = String(u);
    this.href = u;
    var m = u.match(/^([a-z][a-z0-9+.-]*):\/\/([^\/?#]*)/i);
    this.protocol = (m ? m[1] : 'https') + ':';
    this.host = m ? m[2] : '';
    this.hostname = this.host.split(':')[0];
    this.origin = m ? (this.protocol + '//' + this.host) : 'null';
    var rest = u.replace(/^[a-z][a-z0-9+.-]*:\/\/[^\/?#]*/i, '');
    this.pathname = (rest.match(/^[^?#]*/) || [''])[0] || '/';
    this.search = (rest.match(/\?[^#]*/) || [''])[0];
    this.hash = (rest.match(/#.*/) || [''])[0];
  };
}
// `solve()` renames the solver's own `globalThis.location =`, so provide one here.
globalThis.location = new globalThis.URL('https://www.youtube.com/watch?v=yt-dlp-wins');
"#;

struct SolveJob {
    program: String,
    reply: oneshot::Sender<Result<String, String>>,
}

pub struct DenoCoreEngine {
    tx: mpsc::UnboundedSender<SolveJob>,
}

impl Default for DenoCoreEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl DenoCoreEngine {
    /// Spawn the isolate thread; it lives for the process and is reused across solves.
    pub fn new() -> Self {
        // Before spawning, so it can't race the BotGuard isolate (see
        // `ytmusic::ensure_v8_platform`).
        crate::ytmusic::ensure_v8_platform();
        let (tx, rx) = mpsc::unbounded_channel::<SolveJob>();
        std::thread::Builder::new()
            .name("decipher-js".into())
            .spawn(move || run(rx))
            .expect("spawn decipher-js thread");
        Self { tx }
    }
}

impl JsEngine for DenoCoreEngine {
    fn run<'a>(
        &'a self,
        program: String,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send + 'a>>
    {
        let tx = self.tx.clone();
        Box::pin(async move {
            let (reply, rx) = oneshot::channel();
            tx.send(SolveJob { program, reply })
                .map_err(|_| "decipher engine thread gone".to_string())?;
            rx.await
                .map_err(|_| "decipher engine dropped the reply".to_string())?
        })
    }
}

fn run(mut rx: mpsc::UnboundedReceiver<SolveJob>) {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            tracing::error!(error = %e, "decipher: tokio runtime build failed");
            return;
        }
    };
    rt.block_on(async move {
        let mut js = JsRuntime::new(RuntimeOptions {
            extensions: vec![decipher_ext::init_ops()],
            ..Default::default()
        });
        if let Err(e) = js.execute_script("decipher:prelude", PRELUDE) {
            tracing::error!(error = %e, "decipher: prelude failed");
            // Drain with errors so callers don't hang.
            while let Ok(job) = rx.try_recv() {
                let _ = job.reply.send(Err(format!("decipher prelude failed: {e}")));
            }
            return;
        }
        while let Some(job) = rx.recv().await {
            let result = solve_one(&mut js, job.program).await;
            let _ = job.reply.send(result);
        }
    });
}

async fn solve_one(js: &mut JsRuntime, program: String) -> Result<String, String> {
    OUT.with(|o| o.borrow_mut().clear());
    // Solver is synchronous, but drive the event loop so stray microtasks settle.
    let value = js
        .execute_script("decipher:solve", program)
        .map_err(|e| e.to_string())?;
    let resolve = js.resolve(value);
    js.with_event_loop_promise(resolve, PollEventLoopOptions::default())
        .await
        .map_err(|e| e.to_string())?;
    Ok(OUT.with(|o| std::mem::take(&mut *o.borrow_mut())))
}
