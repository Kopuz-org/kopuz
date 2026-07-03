//! Opt-in UI render profiler (`KOPUZ_UI_PROFILE`), wired in [`crate::logging`].
use std::{
    cell::RefCell,
    collections::HashMap,
    fs::{self, File},
    io::{BufWriter, Write as _},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        mpsc::{self, SyncSender},
    },
    thread::{self, JoinHandle},
    time::Instant,
};

use serde_json::{Map, Value};
use tracing::{Subscriber, field::Field, span};
use tracing_subscriber::{Layer, layer::Context, registry::LookupSpan};

/// Per-span data resolved at creation and read back while timing.
struct Tracked {
    /// The component (`scope`) for renders, else the span name.
    name: String,
    /// Tracing target, used as the Chrome-trace category.
    cat: &'static str,
}

struct Frame {
    id: span::Id,
    start: Instant,
    /// Trace timestamp (µs since profiler start) at enter.
    ts_us: f64,
    /// Total time of spans nested inside this one.
    child_ns: u128,
}

thread_local! {
    static STACK: RefCell<Vec<Frame>> = const { RefCell::new(Vec::new()) };
}

#[derive(Default, Clone)]
struct Agg {
    count: u64,
    total_ns: u128,
    self_ns: u128,
    max_self_ns: u128,
}

enum Msg {
    Entry(Value),
    Done,
}

pub struct UiProfileLayer {
    tx: Mutex<SyncSender<Msg>>,
    start: Instant,
    aggs: Arc<Mutex<HashMap<String, Agg>>>,
}

/// Finalizes the trace JSON and writes the summary on drop, and shares the
/// live aggregate map with the layer via `Arc`.
pub struct UiProfileGuard {
    tx: SyncSender<Msg>,
    handle: Option<JoinHandle<()>>,
    aggs: Arc<Mutex<HashMap<String, Agg>>>,
    summary_path: PathBuf,
    start: Instant,
}

impl UiProfileLayer {
    /// `trace_path` gets the Chrome/Perfetto JSON; the summary is written
    /// beside it as `<stem>-summary.txt` when the guard drops.
    pub fn new(trace_path: &Path) -> std::io::Result<(Self, UiProfileGuard)> {
        let file = File::create(trace_path)?;
        let (tx, rx) = mpsc::sync_channel::<Msg>(8192);
        let handle = thread::spawn(move || {
            let mut out = BufWriter::new(file);
            let _ = out.write_all(b"[");
            let mut first = true;
            while let Ok(Msg::Entry(entry)) = rx.recv() {
                if !first {
                    let _ = out.write_all(b",\n");
                }
                first = false;
                let _ = serde_json::to_writer(&mut out, &entry);
            }
            let _ = out.write_all(b"\n]");
            let _ = out.flush();
        });

        let stem = trace_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("kopuz-ui-profile");
        let summary_path = trace_path.with_file_name(format!("{stem}-summary.txt"));

        let start = Instant::now();
        let aggs: Arc<Mutex<HashMap<String, Agg>>> = Arc::default();
        let layer = Self {
            tx: Mutex::new(tx.clone()),
            start,
            aggs: Arc::clone(&aggs),
        };
        let guard = UiProfileGuard {
            tx,
            handle: Some(handle),
            aggs,
            summary_path,
            start,
        };
        Ok((layer, guard))
    }

    fn ts_us(&self) -> f64 {
        self.start.elapsed().as_nanos() as f64 / 1000.0
    }
}

impl<S> Layer<S> for UiProfileLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &span::Attributes<'_>, id: &span::Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        let meta = span.metadata();
        let mut visitor = ScopeVisitor(None);
        attrs.record(&mut visitor);
        let name = visitor.0.unwrap_or_else(|| meta.name().to_string());
        span.extensions_mut().insert(Tracked {
            name,
            cat: meta.target(),
        });
    }

    fn on_enter(&self, id: &span::Id, _ctx: Context<'_, S>) {
        STACK.with(|s| {
            s.borrow_mut().push(Frame {
                id: id.clone(),
                start: Instant::now(),
                ts_us: self.ts_us(),
                child_ns: 0,
            });
        });
    }

    fn on_exit(&self, id: &span::Id, ctx: Context<'_, S>) {
        // Renders nest strictly (LIFO) on the UI thread; bail on any other
        // ordering rather than mis-charge an unrelated frame.
        let Some(frame) = STACK.with(|s| {
            let mut stack = s.borrow_mut();
            match stack.last() {
                Some(top) if top.id == *id => stack.pop(),
                _ => None,
            }
        }) else {
            return;
        };

        let total_ns = frame.start.elapsed().as_nanos();
        let self_ns = total_ns.saturating_sub(frame.child_ns);
        STACK.with(|s| {
            if let Some(parent) = s.borrow_mut().last_mut() {
                parent.child_ns += total_ns;
            }
        });

        let Some(span) = ctx.span(id) else { return };
        let exts = span.extensions();
        let Some(tracked) = exts.get::<Tracked>() else {
            return;
        };

        if let Ok(mut aggs) = self.aggs.lock() {
            let agg = aggs.entry(tracked.name.clone()).or_default();
            agg.count += 1;
            agg.total_ns += total_ns;
            agg.self_ns += self_ns;
            agg.max_self_ns = agg.max_self_ns.max(self_ns);
        }

        let entry = Map::from_iter([
            ("ph".into(), "X".into()),
            ("pid".into(), 1.into()),
            ("tid".into(), 0.into()),
            ("ts".into(), frame.ts_us.into()),
            ("dur".into(), (total_ns as f64 / 1000.0).into()),
            ("name".into(), tracked.name.as_str().into()),
            ("cat".into(), tracked.cat.into()),
        ]);
        if let Ok(tx) = self.tx.lock() {
            let _ = tx.try_send(Msg::Entry(Value::Object(entry)));
        }
    }
}

impl Drop for UiProfileGuard {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Done);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }

        let aggs = self.aggs.lock().map(|a| a.clone()).unwrap_or_default();
        if aggs.is_empty() {
            return;
        }
        let report = render_report(&aggs, self.start.elapsed().as_secs_f64());
        if let Err(e) = fs::write(&self.summary_path, &report) {
            tracing::warn!(
                path = %self.summary_path.display(),
                error = %e,
                "failed to write UI profile summary"
            );
        }
        tracing::info!(summary = %self.summary_path.display(), "\n{}", report);
    }
}

/// Components ranked by total self time, with per-render stats.
fn render_report(aggs: &HashMap<String, Agg>, session_secs: f64) -> String {
    let mut rows: Vec<(&String, &Agg)> = aggs.iter().collect();
    rows.sort_by_key(|(_, agg)| std::cmp::Reverse(agg.self_ns));

    let mut out = format!("UI render profile - {session_secs:.1}s session\n");
    out.push_str(&format!(
        "{:>5}  {:>10}  {:>10}  {:>8}  {:>10}  {:>10}  {}\n",
        "rank", "self ms", "total ms", "renders", "avg self µs", "max self µs", "component"
    ));
    out.push_str(&"-".repeat(90));
    out.push('\n');
    for (i, (name, agg)) in rows.iter().enumerate() {
        let avg_self_us = agg.self_ns as f64 / agg.count.max(1) as f64 / 1000.0;
        out.push_str(&format!(
            "{:>5}  {:>10.2}  {:>10.2}  {:>8}  {:>10.1}  {:>10.1}  {}\n",
            i + 1,
            agg.self_ns as f64 / 1_000_000.0,
            agg.total_ns as f64 / 1_000_000.0,
            agg.count,
            avg_self_us,
            agg.max_self_ns as f64 / 1000.0,
            name
        ));
    }
    out
}

/// Reads the `scope` field off a render span. `%`/`?` values land in
/// `record_debug`; a literal `&str` lands in `record_str`.
struct ScopeVisitor(Option<String>);

impl tracing::field::Visit for ScopeVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "scope" {
            self.0 = Some(value.to_owned());
        }
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "scope" {
            self.0 = Some(format!("{value:?}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::env::temp_dir;
    use std::process;
    use std::time::Duration;

    use super::*;
    use tracing_subscriber::layer::SubscriberExt;

    /// A span nested inside another must have its time subtracted from the
    /// outer span's self time and attributed to itself.
    #[test]
    fn self_time_excludes_nested_spans() {
        let path = temp_dir().join(format!("kopuz-ui-prof-agg-{}.json", process::id()));
        let (layer, guard) = UiProfileLayer::new(&path).unwrap();
        let aggs = Arc::clone(&guard.aggs);

        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            let outer = tracing::info_span!(target: "dioxus_core", "render", scope = "Outer");
            outer.in_scope(|| {
                let inner = tracing::info_span!(target: "dioxus_signals", "recompute");
                inner.in_scope(|| thread::sleep(Duration::from_millis(3)));
            });
        });

        let aggs = aggs.lock().unwrap();
        let outer = &aggs["Outer"];
        let inner = &aggs["recompute"];
        // The outer's only child is `inner`, so this is exact, not timing-dependent.
        assert_eq!(outer.self_ns, outer.total_ns - inner.total_ns);
        assert_eq!(inner.self_ns, inner.total_ns);
        assert!(outer.total_ns >= inner.total_ns);
        drop(aggs);

        drop(guard);
        let _ = fs::remove_file(&path);
    }

    /// Render slices are named by the `scope` field, not the span name.
    #[test]
    fn trace_slices_named_by_scope() {
        let path = temp_dir().join(format!("kopuz-ui-prof-name-{}.json", process::id()));
        let (layer, guard) = UiProfileLayer::new(&path).unwrap();
        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            let s = tracing::info_span!(target: "dioxus_core", "render", scope = "ShowcaseNormal");
            s.in_scope(|| {});
        });
        drop(guard);

        let json: Vec<Value> = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let _ = fs::remove_file(&path);

        let slice = json
            .iter()
            .find(|e| e["ph"] == "X")
            .expect("a complete event");
        assert_eq!(slice["name"], "ShowcaseNormal");
        assert!(slice["dur"].is_number());
    }
}
