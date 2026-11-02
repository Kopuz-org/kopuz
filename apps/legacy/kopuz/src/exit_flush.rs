//! What the app has not sent yet, stashed for the exit paths that run where no signal can be read.

use std::sync::Mutex;
use std::time::Duration;

static STASHED: Mutex<Option<Vec<config::SettingsEdit>>> = Mutex::new(None);
static STASHED_PREFS: Mutex<Option<Vec<api::PrefEntry>>> = Mutex::new(None);

/// How long an exiting process waits for each write; a window that will not close is worse than a lost last-moment change.
const FLUSH_TIMEOUT: Duration = Duration::from_secs(2);

/// Stash the settings edits the daemon has not answered, so an exit write sends only what this app changed.
pub fn stash_config(edits: Vec<config::SettingsEdit>) {
    if let Ok(mut stashed) = STASHED.lock() {
        *stashed = Some(edits);
    }
}

/// Stash the UI preferences not yet sent, for the same exit write.
pub fn stash_prefs(entries: Vec<api::PrefEntry>) {
    if let Ok(mut stashed) = STASHED_PREFS.lock() {
        *stashed = Some(entries);
    }
}

/// Persist unsent edits on a fresh thread: `block_on` panics inside dioxus's tokio context.
pub fn persist_on_fresh_thread(
    edits: Option<Vec<config::SettingsEdit>>,
    prefs: Option<Vec<api::PrefEntry>>,
) {
    let edits = edits.filter(|edits| !edits.is_empty());
    let prefs = prefs.filter(|prefs| !prefs.is_empty());
    if edits.is_none() && prefs.is_none() {
        return;
    }
    let api = crate::backend::api();
    let _ = std::thread::spawn(move || {
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return;
        };
        runtime.block_on(async move {
            if let Some(edits) = edits {
                let write = tokio::time::timeout(FLUSH_TIMEOUT, api.edit_config(edits));
                match write.await {
                    Ok(Err(error)) => tracing::warn!(%error, "config flush on exit failed"),
                    Err(_) => tracing::warn!("config flush on exit timed out"),
                    Ok(Ok(report)) => {
                        let refused = report
                            .outcomes
                            .iter()
                            .filter(|outcome| **outcome != api::EditOutcome::Applied)
                            .count();
                        if refused > 0 {
                            tracing::warn!(refused, "settings refused on exit");
                        }
                    }
                }
            }
            if let Some(prefs) = prefs {
                let frontend = hooks::ui_prefs::FRONTEND.to_string();
                let write =
                    tokio::time::timeout(FLUSH_TIMEOUT, api.set_frontend_prefs(frontend, prefs));
                match write.await {
                    Ok(Err(error)) => tracing::warn!(%error, "UI preferences flush on exit failed"),
                    Err(_) => tracing::warn!("UI preferences flush on exit timed out"),
                    Ok(Ok(())) => {}
                }
            }
        });
    })
    .join();
}

/// Persist unsent edits, flush the core, drop its socket and close the logs; whichever exit path arrives first runs it and the rest wait.
pub fn finish(edits: Option<Vec<config::SettingsEdit>>, prefs: Option<Vec<api::PrefEntry>>) {
    static FINISHED: Mutex<bool> = Mutex::new(false);
    run_once(&FINISHED, || {
        if crate::backend::core().is_some() {
            persist_on_fresh_thread(edits, prefs);
        }
        crate::backend::shutdown();
        crate::logging::shutdown();
    });
}

fn run_once(done: &Mutex<bool>, work: impl FnOnce()) {
    let mut done = done.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if !*done {
        *done = true;
        work();
    }
}

/// Finish with the last stash, for the signal thread and the window teardown, where no signal can be read.
pub fn finish_with_stashed() {
    let edits = STASHED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let prefs = STASHED_PREFS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    finish(edits, prefs);
}

#[cfg(test)]
mod tests {
    use super::run_once;
    use std::sync::Mutex;

    #[test]
    fn every_exit_path_after_the_first_is_a_no_op() {
        let done = Mutex::new(false);
        let mut runs = 0;
        run_once(&done, || runs += 1);
        run_once(&done, || runs += 1);
        assert_eq!(runs, 1);
    }
}
