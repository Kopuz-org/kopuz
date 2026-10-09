//! Getting an expired source session back before anyone is told it expired.
//!
//! A YouTube Music session is a copy of a browser's Google cookies, and the
//! browser keeps rotating the originals. When YouTube stops taking the copy,
//! one `verify_session` rotation or a fresh read of the profile it came from
//! usually has a working one, so a request that answered signed out is worth
//! one repair and one more try.

use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};

/// Lets one repair run at a time. Whoever fails while a repair is running
/// waits for it and takes its answer instead of repairing again.
#[derive(Default)]
pub(crate) struct Recovery {
    /// The last repair's answer.
    last: tokio::sync::Mutex<bool>,
    attempts: AtomicU64,
}

impl Recovery {
    /// Taken before a request runs, and handed to [`Recovery::run`] if it fails.
    pub(crate) fn epoch(&self) -> u64 {
        self.attempts.load(Ordering::Acquire)
    }

    /// Whether the session is usable again. `seen` is the [`Recovery::epoch`]
    /// from before the failed request; a later one means a repair already ran
    /// since, and its answer stands for this failure too.
    pub(crate) async fn run(&self, seen: u64, repair: impl Future<Output = bool>) -> bool {
        let mut last = self.last.lock().await;
        if self.epoch() != seen {
            return *last;
        }
        *last = repair.await;
        self.attempts.fetch_add(1, Ordering::Release);
        *last
    }
}

/// `attempt`, and once more if its answer was `expired` and `repair` got the
/// session back.
pub(crate) async fn once_more<T, A, R>(
    mut attempt: impl FnMut() -> A,
    expired: impl Fn(&T) -> bool,
    repair: impl FnOnce() -> R,
) -> T
where
    A: Future<Output = T>,
    R: Future<Output = bool>,
{
    let first = attempt().await;
    if expired(&first) && repair().await {
        attempt().await
    } else {
        first
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;

    #[tokio::test]
    async fn an_expired_answer_is_tried_once_more_after_a_repair() {
        let attempts = AtomicUsize::new(0);
        let answer = once_more(
            || async { attempts.fetch_add(1, Ordering::SeqCst) },
            |_| true,
            || async { true },
        )
        .await;
        assert_eq!(answer, 1, "the second answer is the one returned");
        assert_eq!(attempts.load(Ordering::SeqCst), 2, "never a third time");
    }

    #[tokio::test]
    async fn a_failed_repair_or_a_live_answer_is_not_tried_again() {
        let attempts = AtomicUsize::new(0);
        once_more(
            || async { attempts.fetch_add(1, Ordering::SeqCst) },
            |_| true,
            || async { false },
        )
        .await;
        once_more(
            || async { attempts.fetch_add(1, Ordering::SeqCst) },
            |_| false,
            || async { panic!("repaired a live session") },
        )
        .await;
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn failures_during_a_repair_share_its_answer() {
        for outcome in [true, false] {
            let recovery = Arc::new(Recovery::default());
            let repairs = Arc::new(AtomicUsize::new(0));
            let seen = recovery.epoch();
            let tasks: Vec<_> = (0..8)
                .map(|_| {
                    let (recovery, repairs) = (recovery.clone(), repairs.clone());
                    tokio::spawn(async move {
                        recovery
                            .run(seen, async {
                                tokio::task::yield_now().await;
                                repairs.fetch_add(1, Ordering::SeqCst);
                                outcome
                            })
                            .await
                    })
                })
                .collect();
            for task in tasks {
                assert_eq!(task.await.expect("task"), outcome);
            }
            assert_eq!(repairs.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn a_later_failure_repairs_again() {
        let recovery = Recovery::default();
        assert!(!recovery.run(recovery.epoch(), async { false }).await);
        assert!(recovery.run(recovery.epoch(), async { true }).await);
        assert_eq!(recovery.epoch(), 2);
    }
}
