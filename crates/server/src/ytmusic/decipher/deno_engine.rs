//! A headless [`JsEngine`] backed by an in-process `deno_core` V8 isolate.
//!
//! This is the decipher engine for the post-WebView world (the Blitz renderer
//! removes the WebView whose JavaScriptCore the old path leaned on). The sig/n
//! transforms are pure computation — no DOM — so a bare `deno_core` isolate runs
//! the vendored `yt_dlp_ejs` solver directly; the only browser globals the
//! solver touches are `print`/`console.log` (captured here) and a `new URL(...)`
//! it assigns to a dead property (shimmed).
//!
//! The isolate is `!Send`, so it lives on one dedicated thread and serves solve
//! programs over a channel. Keeping it alive across tracks amortizes V8 init and
//! lets the JIT warm the sig/n functions.

use std::cell::RefCell;

use deno_core::{JsRuntime, PollEventLoopOptions, RuntimeOptions, op2};
use tokio::sync::{mpsc, oneshot};

use super::JsEngine;

// Per-run capture of what the solver program `print`s. The isolate is
// single-threaded (the actor thread), so a thread-local is the simplest sink.
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

/// Browser globals the `yt_dlp_ejs` solver expects that bare `deno_core` lacks.
/// `print`/`console.log` funnel to the capture op; `URL` is a stub because the
/// solver only assigns `new URL(...)` to a dead property (see `solve`'s
/// `__kopuz_loc` rename) and never reads it back.
const PRELUDE: &str = r#"
globalThis.print = function(s) { Deno.core.ops.op_capture(String(s)); };
globalThis.console = {
  log: globalThis.print, info: globalThis.print,
  warn: globalThis.print, error: globalThis.print, debug: function() {}
};
// Minimal URL: base.js reads .hostname/.origin off URLs and off `location`.
// Bare deno_core has neither; a light parse covers what the solver touches.
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
// `solve()` renames the solver's own `globalThis.location =` (a WebView
// anti-navigation hack), so provide it here for the headless isolate.
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
    /// Spawn the isolate thread and return an engine handle. The thread lives
    /// for the process; the isolate is reused across every solve.
    pub fn new() -> Self {
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
    // The solver program is synchronous, but resolve the completion value
    // through the event loop anyway so any stray microtasks settle.
    let value = js
        .execute_script("decipher:solve", program)
        .map_err(|e| e.to_string())?;
    let resolve = js.resolve(value);
    js.with_event_loop_promise(resolve, PollEventLoopOptions::default())
        .await
        .map_err(|e| e.to_string())?;
    Ok(OUT.with(|o| std::mem::take(&mut *o.borrow_mut())))
}
