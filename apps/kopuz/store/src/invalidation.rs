use std::collections::HashSet;
use std::time::{Duration, Instant};

use api::{ApiEvent, Table};
use gpui_kit::{App, Context, Entity, EventEmitter, Subscription, Task};
use link::Link;

// Scans and syncs invalidate in bursts; this caps refetches at a few per second.
const FLUSH_DELAY: Duration = Duration::from_millis(150);

// Under a stream of invalidations the useful refetch rate is this, not the daemon's event rate.
const MIN_PERIOD: Duration = Duration::from_millis(1000);

/// Tables whose reads went stale; `all` after a resync or a source switch.
#[derive(Debug, Clone, Default)]
pub struct TablesChanged {
    tables: Vec<Table>,
    all: bool,
}

impl TablesChanged {
    pub fn touches(&self, tables: &[Table]) -> bool {
        self.all || tables.iter().any(|table| self.tables.contains(table))
    }

    pub fn is_all(&self) -> bool {
        self.all
    }
}

/// Coalesces `LibraryInvalidated` events into [`TablesChanged`] for queries and models.
pub struct Invalidation {
    dirty: HashSet<Table>,
    flushed_at: Instant,
    flush: Option<Task<()>>,
    _subscription: Subscription,
}

impl EventEmitter<TablesChanged> for Invalidation {}

impl Invalidation {
    pub fn global(cx: &App) -> Entity<Self> {
        crate::handle(cx)
    }

    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let connection = Link::connection(cx);
        let subscription = cx.subscribe(&connection, |this, _, event: &ApiEvent, cx| match event {
            ApiEvent::LibraryInvalidated { table } if *table != Table::Unknown => {
                this.mark(*table, cx)
            }
            ApiEvent::Resync => this.invalidate_all(cx),
            _ => {}
        });
        Self {
            dirty: HashSet::new(),
            flushed_at: Instant::now()
                .checked_sub(MIN_PERIOD)
                .unwrap_or_else(Instant::now),
            flush: None,
            _subscription: subscription,
        }
    }

    /// Mark one table stale, as a local mutation whose event has not arrived yet would.
    pub fn mark(&mut self, table: Table, cx: &mut Context<Self>) {
        self.dirty.insert(table);
        if self.flush.is_none() {
            let wait = FLUSH_DELAY.max(MIN_PERIOD.saturating_sub(self.flushed_at.elapsed()));
            self.flush = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(wait).await;
                this.update(cx, |this, cx| this.flush(cx)).ok();
            }));
        }
    }

    pub fn invalidate_all(&mut self, cx: &mut Context<Self>) {
        // Only here, not per flush: a scan flushes every second and would refetch every failed cover each time.
        crate::artwork::Residents::retry_failed(cx);
        self.dirty.clear();
        self.flush = None;
        self.flushed_at = Instant::now();
        cx.emit(TablesChanged {
            tables: Vec::new(),
            all: true,
        });
    }

    fn flush(&mut self, cx: &mut Context<Self>) {
        self.flush = None;
        self.flushed_at = Instant::now();
        let tables: Vec<Table> = self.dirty.drain().collect();
        if !tables.is_empty() {
            tracing::debug!(?tables, "invalidation flush");
            cx.emit(TablesChanged { tables, all: false });
        }
    }
}
