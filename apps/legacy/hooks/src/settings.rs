//! The app's settings copy: an edit lands in it at once and reaches the daemon as a typed edit once edits settle.

use std::time::Duration;

use config::{SettingKey, Settings, SettingsEdit};
use dioxus::prelude::*;
use tracing::Instrument;

use crate::config_view::LockedKeys;

// A burst of edits, such as keyboard steps on a slider, settles before the daemon is written to.
const SAVE_SETTLE: Duration = Duration::from_millis(600);

#[derive(Clone, Copy)]
pub struct SettingsHandle {
    queue: Queue,
    writer: Coroutine<()>,
}

#[derive(Clone, Copy)]
struct Queue {
    settings: Signal<Settings>,
    /// Edits not sent yet, at most one per field or collection member.
    pending: Signal<Vec<SettingsEdit>>,
    /// Edits sent and not answered yet; a view read meanwhile keeps them on top.
    in_flight: Signal<Vec<SettingsEdit>>,
    revision: Signal<u64>,
    loaded: Signal<bool>,
    locked: Signal<Vec<api::SettingLock>>,
    generation: Signal<u64>,
}

/// Provide the handle over `settings` from the app root, with the locked keys beside it, and send its edits once they settle.
pub fn use_settings_provider(settings: Signal<Settings>) -> SettingsHandle {
    let queue = use_hook(|| Queue {
        settings,
        pending: Signal::new_in_scope(Vec::new(), ScopeId::ROOT),
        in_flight: Signal::new_in_scope(Vec::new(), ScopeId::ROOT),
        revision: Signal::new_in_scope(0, ScopeId::ROOT),
        loaded: Signal::new_in_scope(false, ScopeId::ROOT),
        locked: Signal::new_in_scope(Vec::new(), ScopeId::ROOT),
        generation: Signal::new_in_scope(0, ScopeId::ROOT),
    });
    let writer = crate::settle::use_settled_writer(SAVE_SETTLE, move || queue.flush());
    let handle = SettingsHandle { queue, writer };
    use_context_provider(|| handle);
    use_context_provider(|| LockedKeys(queue.locked));
    handle
}

pub fn use_settings() -> SettingsHandle {
    use_context::<SettingsHandle>()
}

/// Edit through the handle the app root provides.
pub fn edit(edit: SettingsEdit) {
    match try_consume_context::<SettingsHandle>() {
        Some(handle) => handle.edit(edit),
        None => tracing::warn!(
            key = SettingKey::of(&edit).name(),
            "no settings handle; edit dropped"
        ),
    }
}

impl SettingsHandle {
    pub fn settings(self) -> Signal<Settings> {
        self.queue.settings
    }

    /// Apply `edit` to the copy now and queue it for the daemon, replacing a queued edit it supersedes.
    pub fn edit(self, edit: SettingsEdit) {
        let mut queue = self.queue;
        queue.settings.write().apply(edit.clone());
        enqueue(&mut queue.pending.write(), edit);
        self.writer.send(());
    }

    /// Show `edit` in the copy without queueing it, for a slider still being dragged; its release sends it through [`Self::edit`].
    pub fn preview(self, edit: SettingsEdit) {
        let mut settings = self.queue.settings;
        settings.write().apply(edit);
    }

    /// Start from the view the app loaded; edits are only sent after this, so any made before it go now.
    pub fn load(self, view: api::ConfigView) {
        let mut queue = self.queue;
        queue.adopt(view);
        queue.loaded.set(true);
        if !queue.pending.peek().is_empty() {
            self.writer.send(());
        }
    }

    /// Take a daemon view into the copy, keeping every edit it has not answered yet on top.
    pub fn adopt(self, view: api::ConfigView) {
        self.queue.adopt(view);
    }

    /// Re-read what the daemon holds; a later call supersedes one still in flight.
    pub fn reload(self) {
        self.queue.reload();
    }

    /// Every edit the daemon has not answered, for the exit write; none before the first load.
    pub fn unsent(self) -> Vec<SettingsEdit> {
        let queue = self.queue;
        if !*queue.loaded.peek() {
            return Vec::new();
        }
        unsent_of(&queue.in_flight.peek(), &queue.pending.peek())
    }

    /// Like [`Self::unsent`], but re-runs the calling effect when the queue moves.
    pub fn unsent_tracked(self) -> Vec<SettingsEdit> {
        let queue = self.queue;
        if !*queue.loaded.read() {
            return Vec::new();
        }
        unsent_of(&queue.in_flight.read(), &queue.pending.read())
    }
}

impl Queue {
    fn adopt(mut self, view: api::ConfigView) {
        // A read that left before a later write answered is older than what the app holds; a daemon that predates revisions sends 0 and is taken as is.
        if view.revision != 0 && view.revision < *self.revision.peek() {
            return;
        }
        self.revision.set(view.revision);
        if *self.locked.peek() != view.locks {
            self.locked.set(view.locks);
        }
        let mut next = view.settings;
        for edit in self
            .in_flight
            .peek()
            .iter()
            .chain(self.pending.peek().iter())
        {
            next.apply(edit.clone());
        }
        if *self.settings.peek() != next {
            self.settings.set(next);
        }
    }

    fn reload(mut self) {
        let generation = *self.generation.peek() + 1;
        self.generation.set(generation);
        let api = crate::api::consume_api();
        spawn(
            async move {
                match api.config().await {
                    Ok(view) if *self.generation.peek() == generation => self.adopt(view),
                    Ok(_) => {}
                    Err(error) => tracing::warn!(%error, "re-reading settings failed"),
                }
            }
            .instrument(tracing::info_span!("config.reload")),
        );
    }

    /// Send what is queued; false when the daemon could not be reached, the edits having gone back in the queue.
    async fn flush(mut self) -> bool {
        if !*self.loaded.peek() {
            return true;
        }
        let edits = std::mem::take(&mut *self.pending.write());
        if edits.is_empty() {
            return true;
        }
        let api = crate::api::consume_api();
        self.in_flight.set(edits.clone());
        let result = api
            .edit_config(edits.clone())
            .instrument(tracing::info_span!("config.persist"))
            .await;
        self.in_flight.set(Vec::new());
        match result {
            Ok(report) => {
                for (edit, outcome) in edits.iter().zip(&report.outcomes) {
                    let key = SettingKey::of(edit).name();
                    match outcome {
                        api::EditOutcome::Applied => {}
                        api::EditOutcome::Locked => {
                            let by = pinned_by(&report.view.locks, SettingKey::of(edit));
                            tracing::warn!(key, "{by} pins this; reverted")
                        }
                        api::EditOutcome::Invalid { reason } => {
                            tracing::warn!(key, %reason, "the daemon refused this setting; reverted")
                        }
                    }
                }
                self.adopt(report.view);
                true
            }
            Err(error) => {
                tracing::error!(%error, "failed to save settings");
                // Back under any edit made since, so the retry or the exit write sends them.
                let mut pending = self.pending.write();
                let newer = std::mem::take(&mut *pending);
                for edit in edits.into_iter().chain(newer) {
                    enqueue(&mut pending, edit);
                }
                false
            }
        }
    }
}

/// What pins `key` in the view that refused it, for the warning.
fn pinned_by(locks: &[api::SettingLock], key: SettingKey) -> String {
    locks.iter().find(|lock| lock.key == key).map_or_else(
        || "a settings layer".to_string(),
        |lock| lock.by.to_string(),
    )
}

fn enqueue(pending: &mut Vec<SettingsEdit>, edit: SettingsEdit) {
    pending.retain(|queued| !edit.supersedes(queued));
    pending.push(edit);
}

fn unsent_of(in_flight: &[SettingsEdit], pending: &[SettingsEdit]) -> Vec<SettingsEdit> {
    let mut unsent = Vec::new();
    for edit in in_flight.iter().chain(pending) {
        enqueue(&mut unsent, edit.clone());
    }
    unsent
}

#[cfg(test)]
mod tests {
    use super::*;
    use config::{CustomTheme, CustomThemeOp, PlaybackEdit, UiEdit};

    fn crossfade(seconds: u8) -> SettingsEdit {
        SettingsEdit::Playback(PlaybackEdit::CrossfadeSeconds(seconds))
    }

    fn put(name: &str) -> SettingsEdit {
        SettingsEdit::Ui(UiEdit::CustomThemes(CustomThemeOp::Put {
            name: name.into(),
            theme: CustomTheme::default(),
        }))
    }

    fn delete(name: &str) -> SettingsEdit {
        SettingsEdit::Ui(UiEdit::CustomThemes(CustomThemeOp::Delete {
            name: name.into(),
        }))
    }

    #[test]
    fn a_drag_queues_one_edit_and_the_last_value_wins() {
        let mut pending = Vec::new();
        for seconds in 1..=12 {
            enqueue(&mut pending, crossfade(seconds));
        }
        assert_eq!(pending, vec![crossfade(12)]);
    }

    #[test]
    fn ops_on_different_members_both_stay_queued() {
        let mut pending = Vec::new();
        enqueue(&mut pending, put("a"));
        enqueue(&mut pending, put("b"));
        enqueue(&mut pending, crossfade(3));
        assert_eq!(pending.len(), 3);
        enqueue(&mut pending, delete("a"));
        assert_eq!(pending, vec![put("b"), crossfade(3), delete("a")]);
    }

    #[test]
    fn an_unanswered_edit_is_resent_unless_a_newer_one_replaced_it() {
        let unsent = unsent_of(&[crossfade(3), put("a")], &[crossfade(5)]);
        assert_eq!(unsent, vec![put("a"), crossfade(5)]);
    }

    #[test]
    fn a_reverted_edit_names_the_layer_that_pins_it() {
        let key = SettingKey::of(&crossfade(3));
        let lock = |by| api::SettingLock { key, by };
        let dropin = [lock(api::LockSource::DropIn(
            "/x/settings.d/10-playback.toml".into(),
        ))];
        assert_eq!(
            pinned_by(&dropin, key),
            "the drop-in /x/settings.d/10-playback.toml"
        );
        let env = [lock(api::LockSource::Env(
            "KOPUZ_CONFIG_CROSSFADE_SECONDS".into(),
        ))];
        assert_eq!(
            pinned_by(&env, key),
            "the environment variable KOPUZ_CONFIG_CROSSFADE_SECONDS"
        );
        let managed = [lock(api::LockSource::Managed("/x/settings.toml".into()))];
        assert_eq!(
            pinned_by(&managed, key),
            "the managed settings file /x/settings.toml"
        );
    }
}
