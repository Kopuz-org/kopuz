//! Tracing subscriber setup, kept out of `main` so the entrypoint
//! stays readable.
//!
//! Three sinks:
//!   - **console** (stderr, ANSI): live logs for `dx serve` / terminal
//!     runs, at the user filter (default info). Errors/warns always
//!     surface here.
//!   - **file** (daily-rolling, plain): richer (default debug) with
//!     span close + busy/idle timing, for bug reports and offline
//!     analysis. Lives under `<cache>/logs/kopuz.log`.
//!   - **chrome trace** (opt-in via `KOPUZ_TRACE`): a Chrome/Perfetto
//!     trace file for span-level performance + bottleneck analysis.
//!     Off by default → zero overhead.
//!
//! Filter precedence everywhere: `KOPUZ_LOG`, then `RUST_LOG`, then a
//! sensible default. e.g. `KOPUZ_LOG="server::ytmusic=trace,kopuz=debug"`.

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
use std::path::{Path, PathBuf};

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
use tracing_subscriber::{
    EnvFilter, Layer, fmt::format::FmtSpan, layer::SubscriberExt, util::SubscriberInitExt,
};

/// RAII guards that must outlive the program: the file appender's
/// worker thread and (if enabled) the chrome trace flusher. Dropping
/// them flushes both — the daily file's tail and the chrome trace's
/// closing bracket (without which Perfetto can't load it).
///
/// Held in a process global rather than handed to `main` so a Ctrl+C
/// handler can flush them too — `main`'s stack guards never run their
/// Drop on SIGINT, which would leave a truncated, unloadable trace.
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
struct LogGuards {
    _file: tracing_appender::non_blocking::WorkerGuard,
    _chrome: Option<tracing_chrome::FlushGuard>,
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
static GUARDS: std::sync::Mutex<Option<LogGuards>> = std::sync::Mutex::new(None);

/// Quiet the chatty dependencies (and dioxus's per-render memo
/// recompute spans, which otherwise dominate the log) regardless of
/// the base level. Applied as a suffix to every default directive.
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
const QUIET_DEPS: &str = "symphonia=warn,wgpu_core=warn,wgpu_hal=warn,naga=warn,h2=warn,hyper=warn,reqwest=info,cpal=info,sctk=warn,calloop=warn,dioxus_signals=warn,dioxus_core=warn,dioxus_document=warn,zbus=warn,zbus_names=warn,tracing=warn";

/// Base level for the default (no explicit KOPUZ_LOG) case. `info`
/// for ordinary users — keeps the log file small. `KOPUZ_DEBUG=1`
/// bumps it to `debug` for "advanced logs" without forcing the user
/// to hand-write a full KOPUZ_LOG directive (issue #343: ordinary
/// users' disks would fill up fast at debug).
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn default_filter() -> EnvFilter {
    let base = if debug_mode() { "debug" } else { "info" };
    EnvFilter::new(format!("{base},{QUIET_DEPS}"))
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn console_filter() -> EnvFilter {
    user_directives()
        .and_then(|s| EnvFilter::try_new(&s).ok())
        .unwrap_or_else(default_filter)
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn file_filter() -> EnvFilter {
    user_directives()
        .and_then(|s| EnvFilter::try_new(&s).ok())
        .unwrap_or_else(default_filter)
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn debug_mode() -> bool {
    std::env::var("KOPUZ_DEBUG")
        .map(|v| !v.is_empty() && v != "0" && v != "false")
        .unwrap_or(false)
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn user_directives() -> Option<String> {
    std::env::var("KOPUZ_LOG")
        .or_else(|_| std::env::var("RUST_LOG"))
        .ok()
        .filter(|s| !s.is_empty())
}

/// Initialize the global subscriber. Guards are stashed in a process
/// global; call [`shutdown`] on normal exit. A SIGINT handler also
/// flushes them so Ctrl+C still yields a valid trace.
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
pub fn init(log_dir: &Path) {
    let file_appender = tracing_appender::rolling::daily(log_dir, "kopuz.log");
    let (non_blocking, file_guard) = tracing_appender::non_blocking(file_appender);

    let file_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_span_events(FmtSpan::CLOSE)
        .with_writer(non_blocking)
        .with_filter(file_filter());
    let console_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(console_filter());

    let chrome_guard = match std::env::var("KOPUZ_TRACE") {
        // Boolean-style off values disable tracing, matching KOPUZ_DEBUG —
        // otherwise KOPUZ_TRACE=0 would turn it on and write to a file
        // literally named "0". "1"/"true" use the default path; any other
        // non-empty value is treated as an explicit output path.
        Ok(v) if !v.is_empty() && v != "0" && v != "false" => {
            let trace_path = if v == "1" || v == "true" {
                log_dir.join("kopuz-trace.json")
            } else {
                PathBuf::from(v)
            };
            let (chrome_layer, guard) = tracing_chrome::ChromeLayerBuilder::new()
                .file(&trace_path)
                .include_args(true)
                // Async style nests spans by their tracing parent/child
                // relationship on a single track instead of splitting them
                // across per-thread rows. This is what makes an
                // `.instrument()`'d span that hops to a worker thread (e.g.
                // playlist.load_entries -> yt.playlist_entries ->
                // yt.browse_continuation) render as one nested tree you can
                // read without hunting through the worker tracks.
                .trace_style(tracing_chrome::TraceStyle::Async)
                .build();
            tracing_subscriber::registry()
                .with(file_layer)
                .with(console_layer)
                // Filter the chrome layer the same as the file so the
                // trace isn't 30MB of h2/wgpu/dioxus-internal spans
                // burying the kopuz spans you actually want to analyze.
                .with(chrome_layer.with_filter(file_filter()))
                .init();
            tracing::info!(trace = %trace_path.display(), "chrome span trace enabled");
            Some(guard)
        }
        _ => {
            tracing_subscriber::registry()
                .with(file_layer)
                .with(console_layer)
                .init();
            None
        }
    };

    let trace_enabled = chrome_guard.is_some();
    *GUARDS.lock().unwrap() = Some(LogGuards {
        _file: file_guard,
        _chrome: chrome_guard,
    });

    // SIGINT (Ctrl+C from a terminal `cargo run`) skips stack/global
    // Drop, leaving the trace truncated. Flush guards explicitly, then
    // exit with the conventional 130.
    let _ = ctrlc::set_handler(|| {
        shutdown();
        std::process::exit(130);
    });

    // tracing-chrome writes through a BufWriter that only reaches disk on
    // flush or on a clean guard-drop. If the process is killed before the
    // guard drops (hard exit, or a flush race against another exit path),
    // the tail is lost mid-event and the JSON won't parse at all. Flushing
    // on a cadence keeps the on-disk file at complete-event boundaries, so
    // even an ungraceful exit yields a loadable trace — chrome://tracing and
    // Perfetto tolerate a missing trailing `]`, they just can't recover a
    // string cut in half. The clean close (with `]`) still comes from the
    // guard drop on normal exit; this is the backstop.
    if trace_enabled {
        std::thread::spawn(|| {
            loop {
                std::thread::sleep(std::time::Duration::from_millis(500));
                match GUARDS.lock() {
                    Ok(g) => match g.as_ref().and_then(|guards| guards._chrome.as_ref()) {
                        Some(chrome) => chrome.flush(),
                        // Guards were taken on shutdown — nothing left to flush.
                        None => break,
                    },
                    Err(_) => break,
                }
            }
        });
    }

    tracing::info!(log_dir = %log_dir.display(), "logging initialized");
}

/// Flush + drop the logging guards. Idempotent. Called on normal exit
/// and from the SIGINT handler.
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
pub fn shutdown() {
    if let Ok(mut g) = GUARDS.lock() {
        g.take();
    }
}
