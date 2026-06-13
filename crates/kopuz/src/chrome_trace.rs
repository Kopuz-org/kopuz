//! Minimal Chrome trace_event JSON exporter (replaces `tracing-chrome`).
//!
//! `tracing-chrome`'s Async style keys every slice on the ROOT span's
//! tracing id — but tracing-subscriber recycles ids as soon as a span
//! closes, so two visits to the same page yield two same-named roots
//! with the SAME id and trace viewers fuse them into one giant slice
//! bridging the gap. Sharing one id across a subtree has the same
//! flaw in miniature: viewers pair b/e by (cat, id, name), so two
//! concurrently-open same-named siblings cross their begin/end pairs
//! and swap durations. Here every span instance gets its own
//! process-unique id (pairing can never cross), and the hierarchy
//! lives in the name instead — each span is labeled with its full
//! lineage path ("favorites.reconcile › yt.validate › yt.browse"),
//! so the alphabetically sorted track list reads as the span tree.

use std::{
    fs::File,
    io::{BufWriter, Write as _},
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Sender},
    },
    thread::JoinHandle,
    time::Instant,
};

use serde_json::{Map, Value};
use tracing::{Event, Subscriber, field::Field, span};
use tracing_subscriber::{Layer, layer::Context, registry::LookupSpan};

enum Msg {
    Entry(Value),
    Flush,
    Done,
}

pub struct ChromeTraceLayer {
    tx: Mutex<Sender<Msg>>,
    start: Instant,
    next_id: AtomicU64,
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

/// Computed once at span creation; read back at close.
struct SpanInfo {
    path: String,
    id: u64,
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
            next_id: AtomicU64::new(1),
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

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);

        let mut args = Map::new();
        attrs.record(&mut JsonVisitor(&mut args));

        let mut entry = self.entry("b", &path, span.metadata());
        entry["id"] = id.into();
        if !args.is_empty() {
            entry["args"] = Value::Object(args.clone());
        }
        self.send(entry);

        span.extensions_mut().insert(SpanInfo { path, id, args });
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
        entry["id"] = info.id.into();
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
        let path =
            std::env::temp_dir().join(format!("kopuz-chrome-trace-{}.json", std::process::id()));
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

        // Every span instance has its own id — b/e pairing can never cross,
        // even for same-named concurrent siblings or recycled tracing ids.
        let ids: std::collections::HashSet<u64> =
            begins.iter().map(|e| e["id"].as_u64().unwrap()).collect();
        assert_eq!(ids.len(), begins.len());

        assert_eq!(begins[1]["args"]["browse_id"], "VLLM");
        // Every begin closed with a matching end.
        assert_eq!(json.iter().filter(|e| e["ph"] == "e").count(), 4);
    }
}
