use std::cell::Cell;
use std::future::Future;
use std::rc::Rc;
use std::sync::Arc;

use api::{ApiError, KopuzApi, Table};
use futures::FutureExt as _;
use futures::future::BoxFuture;
use gpui_kit::{App, Context, Subscription, Task};
use link::Link;

use crate::invalidation::{Invalidation, TablesChanged};

type Fetch<T> = Rc<dyn Fn(Arc<dyn KopuzApi>) -> BoxFuture<'static, Result<T, ApiError>>>;

/// One daemon read kept fresh: re-runs when its tables are invalidated, keeps the stale value while it does, and drops answers overtaken by a newer run.
pub struct Query<T> {
    fetch: Fetch<T>,
    tables: Vec<Table>,
    value: Option<T>,
    error: Option<ApiError>,
    loading: bool,
    stale: bool,
    generation: u64,
    _task: Option<Task<()>>,
    _subscription: Subscription,
}

impl<T: Send + 'static> Query<T> {
    /// `tables` are the invalidations that re-run `fetch`; a resync or source switch always does.
    pub fn new<Fut>(
        tables: &[Table],
        fetch: impl Fn(Arc<dyn KopuzApi>) -> Fut + 'static,
        cx: &mut Context<Self>,
    ) -> Self
    where
        Fut: Future<Output = Result<T, ApiError>> + Send + 'static,
    {
        let invalidation = Invalidation::global(cx);
        let subscription = cx.subscribe(&invalidation, |this, _, event: &TablesChanged, cx| {
            if event.touches(&this.tables) {
                this.refresh(cx);
            }
        });
        let mut query = Self {
            fetch: box_fetch(fetch),
            tables: tables.to_vec(),
            value: None,
            error: None,
            loading: false,
            stale: false,
            generation: 0,
            _task: None,
            _subscription: subscription,
        };
        if Link::connection(cx).read(cx).is_connected() {
            query.refresh(cx);
        }
        query
    }

    /// Point the query at new inputs, keeping the old value on screen — marked [`Query::is_stale`] — until the new one lands.
    pub fn set_fetch<Fut>(
        &mut self,
        fetch: impl Fn(Arc<dyn KopuzApi>) -> Fut + 'static,
        cx: &mut Context<Self>,
    ) where
        Fut: Future<Output = Result<T, ApiError>> + Send + 'static,
    {
        self.fetch = box_fetch(fetch);
        self.error = None;
        self.stale = self.value.is_some();
        self.refresh(cx);
        // refresh() stays quiet while a value is held, but the inputs just changed: say so.
        if self.stale {
            crate::notify(cx);
        }
    }

    /// Read another window of the same listing: the held rows still answer these inputs, so nothing dims.
    pub fn set_page<Fut>(
        &mut self,
        fetch: impl Fn(Arc<dyn KopuzApi>) -> Fut + 'static,
        cx: &mut Context<Self>,
    ) where
        Fut: Future<Output = Result<T, ApiError>> + Send + 'static,
    {
        self.fetch = box_fetch(fetch);
        self.error = None;
        self.refresh(cx);
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        let fetch = self.fetch.clone();
        let call = Link::global(cx).call(move |api| fetch(api));
        // A held value is already something to show, so only a first read is waited on.
        let first = self.value.is_none().then(|| FirstRead::begin(cx));
        self.loading = true;
        // A value already on screen makes this notify a whole-window rebuild for nothing.
        if self.value.is_none() {
            crate::notify(cx);
        }
        self._task = Some(cx.spawn(async move |this, cx| {
            let result = call.await;
            let _first = first;
            this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                this.stale = false;
                match result {
                    Ok(value) => {
                        this.value = Some(value);
                        this.error = None;
                    }
                    Err(error) => {
                        tracing::debug!(%error, "query failed");
                        // A page reading a failed query renders "nothing here", which reads as an
                        // empty library rather than a refused call. Say so once per new failure;
                        // a daemon outage refreshes every query and would otherwise toast each.
                        let fresh = this.error.as_ref() != Some(&error);
                        // Without an attached daemon the connection banner is what says so.
                        let attached = Link::connection(cx).read(cx).is_connected();
                        this.error = Some(error);
                        if fresh
                            && attached
                            && let Some(error) = this.error.as_ref()
                        {
                            crate::Notices::failed(error, cx);
                        }
                    }
                }
                crate::notify(cx);
            })
            .ok();
        }));
    }
}

impl<T: 'static> Query<T> {
    pub fn value(&self) -> Option<&T> {
        self.value.as_ref()
    }

    pub fn error(&self) -> Option<&ApiError> {
        self.error.as_ref()
    }

    /// A run is in flight; the previous value, if any, is still readable.
    pub fn is_loading(&self) -> bool {
        self.loading
    }

    /// The held value answers the *previous* inputs: dim it rather than blanking the page.
    pub fn is_stale(&self) -> bool {
        self.stale
    }

    /// The first answer (or failure) has arrived.
    pub fn is_settled(&self) -> bool {
        self.value.is_some() || self.error.is_some()
    }

    /// Edit the held value in place, for an optimistic change the next refetch will confirm.
    pub fn update_value(&mut self, cx: &mut Context<Self>, edit: impl FnOnce(&mut T)) {
        if let Some(value) = self.value.as_mut() {
            edit(value);
            crate::notify(cx);
        }
    }
}

fn box_fetch<T, Fut>(fetch: impl Fn(Arc<dyn KopuzApi>) -> Fut + 'static) -> Fetch<T>
where
    Fut: Future<Output = Result<T, ApiError>> + Send + 'static,
{
    Rc::new(move |api| fetch(api).boxed())
}

/// First reads still in flight, so the shell can hold a page up until the one replacing it has something to show.
#[derive(Default)]
struct FirstReads(Rc<Cell<usize>>);

impl gpui_kit::Global for FirstReads {}

/// One first read in flight; dropping it, landed or abandoned, counts it out.
pub struct FirstRead(Rc<Cell<usize>>);

impl FirstRead {
    pub fn begin(cx: &mut App) -> Self {
        Self::counting(cx.default_global::<FirstReads>().0.clone())
    }

    fn counting(count: Rc<Cell<usize>>) -> Self {
        count.set(count.get() + 1);
        Self(count)
    }

    /// How many are in flight now.
    pub fn in_flight(cx: &App) -> usize {
        cx.try_global::<FirstReads>()
            .map_or(0, |reads| reads.0.get())
    }
}

impl Drop for FirstRead {
    fn drop(&mut self) {
        self.0.set(self.0.get().saturating_sub(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_first_read_counts_until_it_is_dropped() {
        let count = Rc::new(Cell::new(0));
        let one = FirstRead::counting(count.clone());
        let two = FirstRead::counting(count.clone());
        assert_eq!(count.get(), 2);
        drop(one);
        assert_eq!(count.get(), 1);
        drop(two);
        assert_eq!(count.get(), 0);
    }
}
