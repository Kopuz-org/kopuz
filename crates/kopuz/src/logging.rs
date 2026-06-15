//! Tracing subscriber setup, kept out of `main` so the entrypoint
//! stays readable.
//!
//! Three sinks:
//!   - **console** (stderr, ANSI): live logs for `dx serve` / terminal
//!     runs, at the user filter (default info). Errors/warns always
//!     surface here.
//!   - **file** (`latest.log`, plain): the current session with span
//!     close + busy/idle timing, for bug reports and offline analysis.
//!     Rotated on startup — the previous session is archived to
//!     `kopuz-<timestamp>.log` (last 10 kept) so a restart never erases a
//!     crashing run. A panic also drops a `crash-<timestamp>.txt`. Files
//!     live under `<cache>/logs/`; see `utils::logs`.
//!   - **chrome trace** (opt-in via the Settings → Logs toggle): a
//!     Chrome/Perfetto trace file for span-level performance + bottleneck
//!     analysis. Off by default → zero overhead.
//!
//! Filter precedence everywhere: `KOPUZ_LOG`, then `RUST_LOG`, then a
//! sensible default. e.g. `KOPUZ_LOG="server::ytmusic=trace,kopuz=debug"`.

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
use std::path::Path;

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
pub fn init(log_dir: &Path, config_tracing_enabled: bool) {
    // Register the dir for crash reports + the export button, then archive the
    // previous session's latest.log (and prune old archives) BEFORE the
    // appender opens a fresh one — so a restart never erases a crashing run.
    utils::logs::set_log_dir(log_dir.to_path_buf());
    utils::logs::rotate_session_log(log_dir);

    let file_appender = tracing_appender::rolling::never(log_dir, "latest.log");
    let (non_blocking, file_guard) = tracing_appender::non_blocking(file_appender);

    let file_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_span_events(FmtSpan::CLOSE)
        .with_writer(non_blocking)
        .with_filter(file_filter());
    let console_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(console_filter());

    // The chrome trace is controlled solely by the in-app settings toggle
    // (`config_tracing_enabled`, read from config at startup) — the UI is the
    // single source of truth, so there's no `KOPUZ_TRACE` env var. Verbosity
    // and filters still come from `KOPUZ_LOG` / `RUST_LOG` / `KOPUZ_DEBUG`.
    let trace_path = log_dir.join("kopuz-trace.json");

    let chrome_guard = if config_tracing_enabled {
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
    } else {
        tracing_subscriber::registry()
            .with(file_layer)
            .with(console_layer)
            .init();
        None
    };

    let trace_enabled = chrome_guard.is_some();
    *GUARDS.lock().unwrap_or_else(|e| e.into_inner()) = Some(LogGuards {
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

    install_panic_hook();

    tracing::info!(log_dir = %log_dir.display(), "logging initialized");
}

/// Chain a panic hook that writes a discrete crash report (panic message +
/// backtrace + recent log tail + version/OS) next to the logs, then defers to
/// the previous hook so the console still shows the panic. Only fires on Rust
/// panics — a hard native crash (SIGSEGV) won't run it.
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn install_panic_hook() {
    use std::sync::atomic::{AtomicBool, Ordering};
    // Only the first panic of the process writes a report. A crash usually
    // cascades — the unwinding main thread trips in-flight worker tasks, which
    // panic too — and without this guard each one would spray its own
    // crash-<timestamp>.txt. The first panic is the root cause; the rest still
    // reach the default hook (console) but don't duplicate the file.
    static CRASH_WRITTEN: AtomicBool = AtomicBool::new(false);
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if !CRASH_WRITTEN.swap(true, Ordering::SeqCst) {
            let message = info
                .payload()
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| info.payload().downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "<non-string panic payload>".to_string());
            let location = info
                .location()
                .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
                .unwrap_or_else(|| "unknown".to_string());
            let backtrace = std::backtrace::Backtrace::force_capture().to_string();

            if let Some(dir) = utils::logs::log_dir()
                && let Some(path) = utils::logs::write_crash_report(
                    &dir,
                    env!("CARGO_PKG_VERSION"),
                    &message,
                    &location,
                    &backtrace,
                )
            {
                tracing::error!(crash_report = %path.display(), panic = %message, %location, "panic — crash report written");
            }
        }
        default(info);
    }));
}

/// Flush + drop the logging guards. Idempotent. Called on normal exit
/// and from the SIGINT handler.
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
pub fn shutdown() {
    if let Ok(mut g) = GUARDS.lock() {
        g.take();
    }
}
