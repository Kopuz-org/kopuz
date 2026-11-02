//! The frontend's profiler: a Chrome trace of the spans, and live counters for what the heap holds.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

mod counters;
mod heap;

pub use counters::{Counters, Snapshot, artwork, elements, images, notifies, rebuilds};
pub use heap::Heap;

/// Kept alive for the process; dropping it is what flushes the trace and the log file.
pub struct Session {
    _chrome: Option<tracing_chrome::FlushGuard>,
    _log: Option<tracing_appender::non_blocking::WorkerGuard>,
    // Held for the process: whoever holds it owns `app-latest.log`.
    _log_lock: Option<std::fs::File>,
}

/// The log file this process writes, which its crash report quotes.
static LOG_NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Installs the log subscriber, writing to stderr and `app-latest.log`, and a Chrome trace layer when `KOPUZ_TRACE` names a file.
///
/// `KOPUZ_TRACE=trace.json kopuz` then opens in <https://ui.perfetto.dev>.
pub fn init() -> Session {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let (file, log_guard, log_lock) = match log_file() {
        Some((writer, guard, lock)) => (
            Some(
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_writer(writer),
            ),
            Some(guard),
            lock,
        ),
        None => (None, None, None),
    };
    let (chrome, chrome_guard) = match std::env::var("KOPUZ_TRACE").ok() {
        Some(path) => {
            let (chrome, guard) = tracing_chrome::ChromeLayerBuilder::new()
                .file(&path)
                .include_args(true)
                .build();
            (Some((chrome, path)), Some(guard))
        }
        None => (None, None),
    };
    let (chrome, trace_path) = chrome.unzip();
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .with(file)
        .with(chrome)
        .init();
    if let Some(path) = trace_path {
        tracing::info!(path, "writing a chrome trace; open it in ui.perfetto.dev");
    }
    install_crash_report();
    Session {
        _chrome: chrome_guard,
        _log: log_guard,
        _log_lock: log_lock,
    }
}

/// `<cache>/kopuz/logs/app-latest.log`, the last session's archived first so a crashing run survives the restart.
fn log_file() -> Option<(
    tracing_appender::non_blocking::NonBlocking,
    tracing_appender::non_blocking::WorkerGuard,
    Option<std::fs::File>,
)> {
    let dir = directories::BaseDirs::new()?
        .cache_dir()
        .join(config::flavor::Flavor::current().name())
        .join("logs");
    std::fs::create_dir_all(&dir).ok()?;
    utils::logs::set_log_dir(dir.clone());
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(dir.join("app-latest.lock"))
        .ok()
        .filter(|lock| lock.try_lock().is_ok());
    // A second window's process logs beside the first rather than archiving its log mid-write.
    let name = match lock {
        Some(_) => {
            utils::logs::rotate_session_log_named(
                &dir,
                utils::logs::APP_LATEST,
                utils::logs::APP_SESSION_PREFIX,
            );
            utils::logs::APP_LATEST.to_string()
        }
        None => format!(
            "{}pid{}.log",
            utils::logs::APP_SESSION_PREFIX,
            std::process::id()
        ),
    };
    let writer = tracing_appender::non_blocking(tracing_appender::rolling::never(&dir, &name));
    let _ = LOG_NAME.set(name);
    Some((writer.0, writer.1, lock))
}

/// A panic leaves a `crash-<time>.txt` beside the log, then unwinds as it would have.
fn install_crash_report() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|text| text.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<non-string panic payload>".to_string());
        let location = info
            .location()
            .map(|at| format!("{}:{}:{}", at.file(), at.line(), at.column()))
            .unwrap_or_else(|| "unknown".to_string());
        let backtrace = std::backtrace::Backtrace::force_capture().to_string();
        tracing::error!(panic = %message, %location, "panic");
        if let Some(dir) = utils::logs::log_dir() {
            utils::logs::write_crash_report_named(
                &dir,
                LOG_NAME
                    .get()
                    .map_or(utils::logs::APP_LATEST, String::as_str),
                &message,
                &location,
                &backtrace,
            );
        }
        default(info);
    }));
}

/// How often the heartbeat reports, since a profiler nobody reads is worth nothing.
pub const HEARTBEAT: Duration = Duration::from_secs(5);

/// One interval's rates, for a reader that wants the numbers rather than a log line.
#[derive(Debug, Clone, Copy, Default)]
pub struct Pulse {
    pub counters: Snapshot,
    pub allocated_mb: u64,
    pub resident_mb: u64,
    pub rebuilds_per_s: f32,
    pub notifies_per_s: f32,
    /// Elements built per rebuild of the window, which is what a frame actually costs.
    pub elements_per_frame: f32,
}

/// Turns the running totals into rates by remembering the last reading; one per reader.
pub struct Meter {
    heap: Heap,
    last: Instant,
    previous: Snapshot,
}

impl Default for Meter {
    fn default() -> Self {
        Self::new()
    }
}

impl Meter {
    pub fn new() -> Self {
        Self {
            heap: Heap::new(),
            last: Instant::now(),
            previous: Counters::snapshot(),
        }
    }

    pub fn sample(&mut self) -> Pulse {
        let counters = Counters::snapshot();
        let (allocated, resident) = self.heap.read();
        // A zero interval would divide by nothing; the first sample can land in the same instant.
        let seconds = self.last.elapsed().as_secs_f32().max(0.001);
        let rebuilds = counters.rebuilds.saturating_sub(self.previous.rebuilds);
        let notifies = counters.notifies.saturating_sub(self.previous.notifies);
        let elements = counters.elements.saturating_sub(self.previous.elements);
        self.last = Instant::now();
        self.previous = counters;
        Pulse {
            counters,
            allocated_mb: allocated / 1_048_576,
            resident_mb: resident / 1_048_576,
            rebuilds_per_s: rebuilds as f32 / seconds,
            notifies_per_s: notifies as f32 / seconds,
            elements_per_frame: elements as f32 / rebuilds.max(1) as f32,
        }
    }
}

/// The costliest views since the last report, worst first, left in place for the next reader.
pub fn worst(limit: usize) -> Vec<(&'static str, SectionStat)> {
    let mut sections: Vec<(&'static str, SectionStat)> = {
        let held = SECTIONS.lock().unwrap_or_else(|held| held.into_inner());
        held.iter().map(|(name, stat)| (*name, *stat)).collect()
    };
    sections.sort_unstable_by_key(|(_, stat)| std::cmp::Reverse(stat.total_us));
    sections.truncate(limit);
    sections
}

/// Reports the heap and the counters; drive it from a timer, not from a render.
pub struct Heartbeat {
    meter: Meter,
}

impl Default for Heartbeat {
    fn default() -> Self {
        Self::new()
    }
}

impl Heartbeat {
    pub fn new() -> Self {
        Self {
            meter: Meter::new(),
        }
    }

    /// Logs one line of what the process is holding, and the four costliest views.
    pub fn report(&mut self) {
        let pulse = self.meter.sample();
        let counters = pulse.counters;
        tracing::info!(
            allocated_mb = pulse.allocated_mb,
            resident_mb = pulse.resident_mb,
            artwork_live = counters.artwork_live,
            artwork_tracked = counters.artwork_tracked,
            artwork_loaded = counters.artwork_loaded,
            artwork_released = counters.artwork_released,
            artwork_bytes_mb = counters.artwork_bytes / 1_048_576,
            artwork_decoded_mb = counters.artwork_decoded / 1_048_576,
            rebuilds = counters.rebuilds,
            rebuilds_per_s = pulse.rebuilds_per_s,
            notifies_per_s = pulse.notifies_per_s,
            elements_per_frame = pulse.elements_per_frame,
            "profile"
        );
        let sections = worst(4);
        // Cleared after reading, so each line covers one interval rather than the whole run.
        SECTIONS
            .lock()
            .unwrap_or_else(|held| held.into_inner())
            .clear();
        for (name, stat) in sections {
            tracing::info!(
                view = name,
                renders = stat.calls,
                total_ms = stat.total_us as f32 / 1000.,
                worst_ms = stat.worst_us as f32 / 1000.,
                "render"
            );
        }
    }
}

/// Counts how often the window's whole view tree is rebuilt.
///
/// This is NOT frame time. GPUI renders a child during the parent's prepaint, so a guard taken
/// in the shell's `render` is dropped before any page has rendered, let alone been laid out or
/// painted. What it does measure is the thing worth watching: any entity notifying marks its
/// ancestors dirty, so one 250ms playhead tick rebuilds every visible view. A rate far above
/// the tick rate means something is notifying in a loop.
pub struct Rebuild;

impl Default for Rebuild {
    fn default() -> Self {
        Self::new()
    }
}

impl Rebuild {
    pub fn new() -> Self {
        REBUILDS.fetch_add(1, Ordering::Relaxed);
        Self
    }
}

/// Times one view's `render`, which is where the work actually is.
///
/// `profile::render("albums")` at the top of a `Render::render`; the guard records on drop.
pub fn render(name: &'static str) -> Section {
    Section {
        name,
        started: Instant::now(),
    }
}

pub struct Section {
    name: &'static str,
    started: Instant,
}

impl Drop for Section {
    fn drop(&mut self) {
        let micros = self.started.elapsed().as_micros() as u64;
        let mut sections = SECTIONS.lock().unwrap_or_else(|held| held.into_inner());
        let stat = sections.entry(self.name).or_default();
        stat.calls += 1;
        stat.total_us += micros;
        stat.worst_us = stat.worst_us.max(micros);
    }
}

/// What one named render has cost since the last report.
#[derive(Debug, Clone, Copy, Default)]
pub struct SectionStat {
    pub calls: u64,
    pub total_us: u64,
    pub worst_us: u64,
}

pub(crate) static SECTIONS: std::sync::Mutex<
    std::collections::BTreeMap<&'static str, SectionStat>,
> = std::sync::Mutex::new(std::collections::BTreeMap::new());

pub(crate) static REBUILDS: AtomicU64 = AtomicU64::new(0);
pub(crate) static NOTIFIES: AtomicU64 = AtomicU64::new(0);
pub(crate) static ELEMENTS: AtomicU64 = AtomicU64::new(0);

pub(crate) static ARTWORK_LIVE: AtomicUsize = AtomicUsize::new(0);
pub(crate) static ARTWORK_BYTES: AtomicU64 = AtomicU64::new(0);
pub(crate) static ARTWORK_DECODED: AtomicU64 = AtomicU64::new(0);

pub(crate) static ARTWORK_LOADED: AtomicU64 = AtomicU64::new(0);
pub(crate) static ARTWORK_RELEASED: AtomicU64 = AtomicU64::new(0);

pub(crate) static ARTWORK_TRACKED: AtomicUsize = AtomicUsize::new(0);
