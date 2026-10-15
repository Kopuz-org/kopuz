//! One writer for state edited in bursts: a change wakes it, every further change restarts the settle window, and it writes once the window passes quietly.

use std::future::Future;
use std::time::Duration;

use dioxus::prelude::*;
use futures_util::{Stream, StreamExt};

/// The longest a failed write waits before it is tried again.
const RETRY_CAP: Duration = Duration::from_secs(30);

/// Start the writer and return what wakes it; `write` answers whether the state reached its store.
pub fn use_settled_writer<W, F>(settle: Duration, write: W) -> Coroutine<()>
where
    W: FnMut() -> F + 'static,
    F: Future<Output = bool> + 'static,
{
    let mut write = Some(write);
    use_coroutine(move |changes: UnboundedReceiver<()>| {
        let write = write.take();
        async move {
            if let Some(write) = write {
                run(changes, settle, write).await;
            }
        }
    })
}

/// Write once per quiet spell; a write that failed is tried again after a growing pause, the state having stayed queued.
async fn run<S, W, F>(mut changes: S, settle: Duration, mut write: W)
where
    S: Stream<Item = ()> + Unpin,
    W: FnMut() -> F,
    F: Future<Output = bool>,
{
    while changes.next().await.is_some() {
        let mut retry = settle;
        loop {
            while let Ok(Some(())) = tokio::time::timeout(settle, changes.next()).await {}
            if write().await {
                break;
            }
            tokio::time::sleep(retry).await;
            retry = (retry * 2).min(RETRY_CAP);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    const SETTLE: Duration = Duration::from_millis(600);

    fn changes() -> (
        tokio::sync::mpsc::UnboundedSender<()>,
        impl Stream<Item = ()> + Unpin,
    ) {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        (
            tx,
            futures_util::stream::poll_fn(move |cx| rx.poll_recv(cx)),
        )
    }

    type Writes = Rc<RefCell<Vec<tokio::time::Instant>>>;

    fn recorder(results: Vec<bool>) -> (Writes, impl FnMut() -> std::future::Ready<bool>) {
        let writes = Rc::new(RefCell::new(Vec::new()));
        let mut results = results.into_iter();
        let log = writes.clone();
        let write = move || {
            log.borrow_mut().push(tokio::time::Instant::now());
            std::future::ready(results.next().unwrap_or(true))
        };
        (writes, write)
    }

    #[tokio::test(start_paused = true)]
    async fn a_burst_is_written_once_after_it_settles() {
        let (tx, rx) = changes();
        let (writes, write) = recorder(Vec::new());
        let start = tokio::time::Instant::now();
        let feed = async move {
            for _ in 0..5 {
                tx.send(()).unwrap();
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        };
        tokio::join!(run(rx, SETTLE, write), feed);
        let writes = writes.borrow();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0] - start, Duration::from_millis(800) + SETTLE);
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_write_is_tried_again_without_a_new_change() {
        let (tx, rx) = changes();
        let (writes, write) = recorder(vec![false, false, true]);
        let feed = async move {
            tx.send(()).unwrap();
            tokio::time::sleep(Duration::from_secs(10)).await;
        };
        tokio::join!(run(rx, SETTLE, write), feed);
        assert_eq!(writes.borrow().len(), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn nothing_is_written_without_a_change() {
        let (tx, rx) = changes();
        let (writes, write) = recorder(Vec::new());
        drop(tx);
        run(rx, SETTLE, write).await;
        assert!(writes.borrow().is_empty());
    }
}
