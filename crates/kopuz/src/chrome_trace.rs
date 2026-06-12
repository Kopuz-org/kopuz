//! Minimal Chrome trace_event JSON exporter (replaces `tracing-chrome`).
//!
//! `tracing-chrome`'s Async style keys every slice on the ROOT span's
//! tracing id — but tracing-subscriber recycles ids as soon as a span
//! closes, so two visits to the same page yield two same-named roots
//! with the SAME id and trace viewers fuse them into one giant slice
//! bridging the gap. Here every root gets a process-unique monotonic
//! instance id that its whole subtree inherits, and every span is
//! named by its full lineage path ("favorites.reconcile › yt.validate
//! › yt.browse"), so no viewer grouping rule (by name, by id, or
//! both) can merge unrelated trees — and the alphabetically sorted
//! track list reads as the span hierarchy.

use std::{
    fs::File,
    io::{BufWriter, Write as _},
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Sender},
        Mutex,
    },
    thread::JoinHandle,
    time::Instant,
};

use serde_json::{Map, Value};
use tracing::{field::Field, span, Event, Subscriber};
use tracing_subscriber::{layer::Context, registry::LookupSpan, Layer};

enum Msg {
    Entry(Value),
    Flush,
    Done,
}

pub struct ChromeTraceLayer {
    tx: Mutex<Sender<Msg>>,
    start: Instant,
    next_root: AtomicU64,
}

/// Finalizes the JSON array on drop — hold it for the app's lifetime.
pub struct FlushGuard {
    tx: Sender<Msg>,
    handle: Option<JoinHandle<()>>,
}

impl FlushGuard {
    /// Push buffered entries to disk without finalizing the array —
    /// the file always ends at a complete-event boundary, so a hard
    /// kill still leaves a loadable trace (viewers tolerate the
    /// missing trailing `]`).
    pub fn flush(&self) {
        let _ = self.tx.send(Msg::Flush);
    }
}

impl Drop for FlushGuard {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Done);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Process-unique instance number, stored on each root span's extensions.
struct RootInstance(u64);

/// Computed once at span creation; read back at close.
struct SpanInfo {
    path: String,
    root: u64,
    args: Map<String, Value>,
}

impl ChromeTraceLayer {
    pub fn new(path: &Path) -> std::io::Result<(Self, FlushGuard)> {
        let file = File::create(path)?;
        let (tx, rx) = mpsc::channel::<Msg>();
        let handle = std::thread::spawn(move || {
            let mut out = BufWriter::new(file);
            let _ = out.write_all(b"[");
            let mut first = true;
            loop {
                match rx.recv() {
                    Ok(Msg::Entry(entry)) => {
                        if !first {
                            let _ = out.write_all(b",\n");
                        }
                        first = false;
                        let _ = serde_json::to_writer(&mut out, &entry);
                    }
                    Ok(Msg::Flush) => {
                        let _ = out.flush();
                    }
                    Ok(Msg::Done) | Err(_) => break,
                }
            }
            let _ = out.write_all(b"\n]");
            let _ = out.flush();
        });
        let layer = Self {
            tx: Mutex::new(tx.clone()),
            start: Instant::now(),
            next_root: AtomicU64::new(1),
        };
        let guard = FlushGuard {
            tx,
            handle: Some(handle),
        };
        Ok((layer, guard))
    }

    fn ts(&self) -> f64 {
        self.start.elapsed().as_nanos() as f64 / 1000.0
    }

    fn send(&self, entry: Value) {
        if let Ok(tx) = self.tx.lock() {
            let _ = tx.send(Msg::Entry(entry));
        }
    }

    fn entry(&self, ph: &str, name: &str, meta: &tracing::Metadata<'_>) -> Value {
        let mut entry = Map::new();
        entry.insert("ph".into(), ph.into());
        entry.insert("pid".into(), 1.into());
        entry.insert("tid".into(), 0.into());
        entry.insert("ts".into(), self.ts().into());
        entry.insert("name".into(), name.into());
        entry.insert("cat".into(), meta.target().into());
        if let (Some(file), Some(line)) = (meta.file(), meta.line()) {
            entry.insert(".file".into(), file.into());
            entry.insert(".line".into(), line.into());
        }
        Value::Object(entry)
    }
}

impl<S> Layer<S> for ChromeTraceLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &span::Attributes<'_>, id: &span::Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };

        // Lineage walks the full registry scope, so ancestors filtered
        // out of this layer still show up in the path.
        let mut path = String::new();
        for ancestor in span.scope().from_root() {
            if !path.is_empty() {
                path.push_str(" › ");
            }
            path.push_str(ancestor.name());
        }

        let root = {
            let root_ref = span
                .scope()
                .from_root()
                .next()
                .expect("scope contains at least self");
            let mut exts = root_ref.extensions_mut();
            match exts.get_mut::<RootInstance>() {
                Some(inst) => inst.0,
                None => {
                    let n = self.next_root.fetch_add(1, Ordering::Relaxed);
                    exts.insert(RootInstance(n));
                    n
                }
            }
        };

        let mut args = Map::new();
        attrs.record(&mut JsonVisitor(&mut args));

        let mut entry = self.entry("b", &path, span.metadata());
        entry["id"] = root.into();
        if !args.is_empty() {
            entry["args"] = Value::Object(args.clone());
        }
        self.send(entry);

        span.extensions_mut().insert(SpanInfo { path, root, args });
    }

    fn on_record(&self, id: &span::Id, values: &span::Record<'_>, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        if let Some(info) = span.extensions_mut().get_mut::<SpanInfo>() {
            values.record(&mut JsonVisitor(&mut info.args));
        }
    }

    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut args = Map::new();
        event.record(&mut JsonVisitor(&mut args));
        let mut entry = self.entry("i", event.metadata().name(), event.metadata());
        entry["s"] = "t".into();
        if !args.is_empty() {
            entry["args"] = Value::Object(args);
        }
        self.send(entry);
    }

    fn on_close(&self, id: span::Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(&id) else { return };
        let exts = span.extensions();
        // Absent when the span predates this layer or was filtered out.
        let Some(info) = exts.get::<SpanInfo>() else {
            return;
        };
        let mut entry = self.entry("e", &info.path, span.metadata());
        entry["id"] = info.root.into();
        if !info.args.is_empty() {
            entry["args"] = Value::Object(info.args.clone());
        }
        self.send(entry);
    }
}

struct JsonVisitor<'a>(&'a mut Map<String, Value>);

impl tracing::field::Visit for JsonVisitor<'_> {
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.0.insert(field.name().to_owned(), value.into());
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.0.insert(field.name().to_owned(), value.into());
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.insert(field.name().to_owned(), value.into());
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.0.insert(field.name().to_owned(), value.into());
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.into());
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().to_owned(), format!("{value:?}").into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::layer::SubscriberExt;

    #[test]
    fn repeat_roots_get_distinct_ids_and_lineage_names() {
        let path = std::env::temp_dir().join(format!("kopuz-chrome-trace-{}.json", std::process::id()));
        let (layer, guard) = ChromeTraceLayer::new(&path).unwrap();
        let subscriber = tracing_subscriber::registry().with(layer);

        tracing::subscriber::with_default(subscriber, || {
            for _ in 0..2 {
                let root = tracing::info_span!("favorites.reconcile");
                let _root = root.enter();
                let child = tracing::info_span!("yt.browse", browse_id = "VLLM");
                let _child = child.enter();
            }
        });
        drop(guard);

        let json: Vec<Value> =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let _ = std::fs::remove_file(&path);

        let begins: Vec<&Value> = json.iter().filter(|e| e["ph"] == "b").collect();
        let names: Vec<&str> = begins.iter().map(|e| e["name"].as_str().unwrap()).collect();
        assert_eq!(
            names,
            [
                "favorites.reconcile",
                "favorites.reconcile › yt.browse",
                "favorites.reconcile",
                "favorites.reconcile › yt.browse",
            ]
        );

        // Children inherit their root's instance id…
        assert_eq!(begins[0]["id"], begins[1]["id"]);
        assert_eq!(begins[2]["id"], begins[3]["id"]);
        // …and the two visits never share one, even though tracing
        // recycles the underlying span ids.
        assert_ne!(begins[0]["id"], begins[2]["id"]);

        assert_eq!(begins[1]["args"]["browse_id"], "VLLM");
        // Every begin closed with a matching end.
        assert_eq!(json.iter().filter(|e| e["ph"] == "e").count(), 4);
    }
}
