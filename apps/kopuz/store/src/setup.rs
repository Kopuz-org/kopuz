//! Whether first-run setup has been done, as the daemon's `gpui` preferences remember it.

use api::{ApiEvent, FrontendPref, PrefEntry};
use gpui_kit::{App, Context, Entity, Subscription, Task};
use link::Link;

use crate::layout::FRONTEND;

/// The preference key, holding `"true"` or `"false"`.
pub const KEY: &str = "setup_complete";

/// Only an explicit `"true"` counts: missing or unreadable means setup still has to run.
pub fn parse(raw: Option<&str>) -> bool {
    raw.is_some_and(|value| value.trim() == "true")
}

/// What a first read of the stored preferences decides.
pub fn decide(stored: &[FrontendPref]) -> Decision {
    let raw = stored
        .iter()
        .find(|pref| pref.key == KEY)
        .map(|pref| pref.value.as_str());
    match parse(raw) {
        true => Decision::Complete,
        false => Decision::Guide,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// No successful read yet, so nothing may be decided.
    Unknown,
    Guide,
    Complete,
}

/// The first-run decision and whether the guide is on screen.
pub struct SetupModel {
    decision: Decision,
    showing: bool,
    /// Bumped each time the guide opens, so it starts over instead of resuming a stale step.
    run: u64,
    /// The guide opened by itself, for a setup that never ran, rather than because someone asked.
    unprompted: bool,
    _load: Option<Task<()>>,
    _subscription: Subscription,
}

impl SetupModel {
    pub fn global(cx: &App) -> Entity<Self> {
        crate::handle(cx)
    }

    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let connection = Link::connection(cx);
        let subscription = cx.subscribe(&connection, |this, _, event: &ApiEvent, cx| {
            if matches!(event, ApiEvent::Resync) {
                this.read(cx);
            }
        });
        Self {
            decision: Decision::Unknown,
            showing: false,
            run: 0,
            unprompted: false,
            _load: None,
            _subscription: subscription,
        }
    }

    pub fn decision(&self) -> Decision {
        self.decision
    }

    pub fn is_decided(&self) -> bool {
        self.decision != Decision::Unknown
    }

    pub fn is_complete(&self) -> bool {
        self.decision == Decision::Complete
    }

    /// Whether the shell draws the guide instead of the page.
    pub fn showing(&self) -> bool {
        self.showing
    }

    pub fn run(&self) -> u64 {
        self.run
    }

    /// Whether this run is a first setup, which may set defaults, rather than one someone reopened.
    pub fn unprompted(&self) -> bool {
        self.unprompted
    }

    /// Ask again; a decision already taken stands, so this only matters before the first answer.
    pub fn read(&mut self, cx: &mut Context<Self>) {
        if self.is_decided() {
            return;
        }
        let call = Link::global(cx)
            .call(|api| async move { api.frontend_prefs(FRONTEND.to_string()).await });
        self._load = Some(cx.spawn(async move |this, cx| {
            let stored = match call.await {
                Ok(stored) => stored,
                Err(error) => {
                    tracing::warn!(%error, "could not read whether setup is complete");
                    return;
                }
            };
            this.update(cx, |this, cx| this.adopt(decide(&stored), cx))
                .ok();
        }));
    }

    fn adopt(&mut self, decision: Decision, cx: &mut Context<Self>) {
        if self.is_decided() {
            return;
        }
        self.decision = decision;
        if decision == Decision::Guide {
            self.unprompted = true;
            self.open(cx);
        }
        crate::notify(cx);
    }

    fn open(&mut self, cx: &mut Context<Self>) {
        self.showing = true;
        self.run += 1;
        crate::notify(cx);
    }

    /// Setup is done; the guide may stay up to wrap up.
    pub fn complete(&mut self, cx: &mut Context<Self>) {
        self.decision = Decision::Complete;
        store(true, cx);
        crate::notify(cx);
    }

    /// Take the guide down.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.showing = false;
        crate::notify(cx);
    }

    /// Skipping counts as done, so the guide does not come back next start.
    pub fn skip(&mut self, cx: &mut Context<Self>) {
        self.complete(cx);
        self.close(cx);
    }

    /// Forget setup was done and open the guide from its first step.
    pub fn run_again(&mut self, cx: &mut Context<Self>) {
        self.decision = Decision::Guide;
        self.unprompted = false;
        store(false, cx);
        self.open(cx);
    }

    /// Open the guide from anywhere holding an `App`.
    pub fn start_over(cx: &mut App) {
        Self::global(cx).update(cx, |setup, cx| setup.run_again(cx));
    }
}

fn store(complete: bool, cx: &mut App) {
    let entry = PrefEntry {
        key: KEY.into(),
        value: Some(complete.to_string()),
    };
    Link::global(cx).fire(cx, "store setup_complete", move |api| async move {
        api.set_frontend_prefs(FRONTEND.to_string(), vec![entry])
            .await
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pref(key: &str, value: &str) -> FrontendPref {
        FrontendPref {
            key: key.into(),
            value: value.into(),
        }
    }

    #[test]
    fn only_true_means_done() {
        assert!(parse(Some("true")));
        assert!(parse(Some(" true\n")));
        assert!(!parse(Some("false")));
        assert!(!parse(Some("yes")));
        assert!(!parse(Some("")));
        assert!(!parse(None));
    }

    #[test]
    fn a_missing_key_opens_the_guide() {
        assert_eq!(decide(&[]), Decision::Guide);
        assert_eq!(decide(&[pref("skin", "\"normal\"")]), Decision::Guide);
    }

    #[test]
    fn a_stored_answer_decides() {
        assert_eq!(decide(&[pref(KEY, "true")]), Decision::Complete);
        assert_eq!(decide(&[pref(KEY, "false")]), Decision::Guide);
        assert_eq!(decide(&[pref(KEY, "garbage")]), Decision::Guide);
    }
}
