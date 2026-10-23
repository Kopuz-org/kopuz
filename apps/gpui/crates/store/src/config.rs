use std::time::Duration;

use api::{ApiEvent, ConfigView, EditOutcome, EditReport, LockSource, SettingLock};
use config::{SettingKey, Settings, SettingsEdit};
use gpui_kit::{App, Context, Entity, SharedString, Subscription, Task};
use link::Link;

use crate::notices::Notices;

// Slider drags and typing settle before a write; the Dioxus app used the same delay.
const SAVE_DELAY: Duration = Duration::from_millis(600);

// How long a settings row says "Saved" before going quiet again.
const SAVED_FOR: Duration = Duration::from_millis(2200);

/// The daemon's settings, edited locally at once and sent back debounced as typed edits.
pub struct ConfigModel {
    view: ConfigView,
    loaded: bool,
    /// Edits not sent yet, at most one per field or collection member.
    pending: Vec<SettingsEdit>,
    /// Edits sent and not answered yet; a view read meanwhile keeps them on top.
    in_flight: Vec<SettingsEdit>,
    saved: Vec<SettingKey>,
    _save: Option<Task<()>>,
    _tick: Option<Task<()>>,
    _fetch: Option<Task<()>>,
    _subscription: Subscription,
}

impl ConfigModel {
    pub fn global(cx: &App) -> Entity<Self> {
        crate::handle(cx)
    }

    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let connection = Link::connection(cx);
        let subscription = cx.subscribe(&connection, |this, _, event: &ApiEvent, cx| match event {
            ApiEvent::Resync => this.reload(cx),
            ApiEvent::ConfigChanged { revision } if *revision > this.view.revision => {
                this.reload(cx)
            }
            _ => {}
        });
        Self {
            view: ConfigView::default(),
            loaded: false,
            pending: Vec::new(),
            in_flight: Vec::new(),
            saved: Vec::new(),
            _save: None,
            _tick: None,
            _fetch: None,
            _subscription: subscription,
        }
    }

    pub fn config(&self) -> &Settings {
        &self.view.settings
    }

    /// Whether track rows carry their cover, which every list of tracks asks.
    pub fn row_images(&self) -> bool {
        self.view.settings.ui.show_row_images
    }

    /// Nothing is saved until the daemon's config has been read, so a default is never persisted.
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    pub fn locks(&self) -> &[SettingLock] {
        &self.view.locks
    }

    /// The stored names of every pinned key, as a daemon field spec names them.
    pub fn locked_names(&self) -> Vec<String> {
        self.view
            .locks
            .iter()
            .map(|lock| lock.key.name().to_string())
            .collect()
    }

    /// The layer pinning the key stored under `name`.
    pub fn lock_of(&self, name: &str) -> Option<&LockSource> {
        self.view
            .locks
            .iter()
            .find(|lock| lock.key.name() == name)
            .map(|lock| &lock.by)
    }

    /// A settings layer pins `name`; its control renders locked.
    pub fn is_locked(&self, name: &str) -> bool {
        self.lock_of(name).is_some()
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let call = Link::global(cx).call(|api| async move { api.config().await });
        self._fetch = Some(cx.spawn(async move |this, cx| match call.await {
            Ok(view) => {
                this.update(cx, |this, cx| this.adopt(view, cx)).ok();
            }
            Err(error) => tracing::warn!(%error, "could not read the daemon config"),
        }));
    }

    /// Whether a write for the key stored under `name` has just landed, which is what a row's "Saved" tick reads.
    pub fn just_saved(&self, name: &str) -> bool {
        self.saved.iter().any(|key| key.name() == name)
    }

    /// A write is on its way out or in flight.
    pub fn is_saving(&self) -> bool {
        self._save.is_some()
    }

    /// Change a setting from anywhere holding an `App`.
    pub fn edit(cx: &mut App, edit: SettingsEdit) {
        Self::global(cx).update(cx, |model, cx| model.update(cx, edit));
    }

    /// What a settings control calls with the value it picked.
    pub fn writer<T: 'static>(
        edit: impl Fn(T) -> SettingsEdit + 'static,
    ) -> impl Fn(T, &mut App) + 'static {
        move |value, cx| Self::edit(cx, edit(value))
    }

    /// Apply `edit` to the copy now and queue it; the daemon hears it once edits settle.
    pub fn update(&mut self, cx: &mut Context<Self>, edit: SettingsEdit) {
        if !self.loaded {
            tracing::warn!(
                key = SettingKey::of(&edit).name(),
                "config edit before the daemon config loaded; ignored"
            );
            return;
        }
        let key = SettingKey::of(&edit);
        self.saved.retain(|held| *held != key);
        let language = self.view.settings.ui.language.clone();
        self.view.settings.apply(edit.clone());
        if self.view.settings.ui.language != language {
            i18n::set_locale(&self.view.settings.ui.language);
        }
        queue(&mut self.pending, edit);
        crate::notify(cx);
        self.schedule_save(cx);
    }

    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        self._save = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            let Ok(call) = this.update(cx, |this, cx| {
                let edits = std::mem::take(&mut this.pending);
                this.in_flight = edits.clone();
                (!edits.is_empty()).then(|| {
                    let sent = edits.clone();
                    let call = Link::global(cx)
                        .call(move |api| async move { api.edit_config(sent).await });
                    (edits, call)
                })
            }) else {
                return;
            };
            let Some((edits, call)) = call else {
                this.update(cx, |this, _| this._save = None).ok();
                return;
            };
            let result = call.await;
            this.update(cx, |this, cx| {
                this._save = None;
                this.in_flight.clear();
                match result {
                    Ok(report) => this.answer(&edits, report, cx),
                    // The control is showing an edit the daemon never took; put the truth back.
                    Err(error) => {
                        Notices::failed(&error, cx);
                        this.reload(cx);
                    }
                }
                if !this.pending.is_empty() {
                    this.schedule_save(cx);
                }
            })
            .ok();
        }));
    }

    /// Take the daemon's answer: a refused edit is reverted by adopting its view, and said out loud.
    fn answer(&mut self, edits: &[SettingsEdit], report: EditReport, cx: &mut Context<Self>) {
        let mut applied = Vec::new();
        for (edit, outcome) in edits.iter().zip(&report.outcomes) {
            let key = SettingKey::of(edit);
            match outcome {
                EditOutcome::Applied => applied.push(key),
                EditOutcome::Locked => {
                    tracing::warn!(key = key.name(), "a settings layer pins this; reverted");
                    let cause = report
                        .view
                        .locks
                        .iter()
                        .find(|lock| lock.key == key)
                        .map(|lock| lock_cause(&lock.by))
                        .unwrap_or_else(|| i18n::t("setting_managed_by_system"));
                    Notices::error(SharedString::from(cause), cx);
                }
                EditOutcome::Invalid { reason } => {
                    tracing::warn!(key = key.name(), %reason, "the daemon refused this setting; reverted");
                    Notices::error(SharedString::from(reason.clone()), cx);
                }
            }
        }
        self.adopt(report.view, cx);
        self.confirm(applied, cx);
    }

    /// Mark what the write carried as saved, and take the mark off again a moment later.
    fn confirm(&mut self, applied: Vec<SettingKey>, cx: &mut Context<Self>) {
        self.saved = applied;
        if self.saved.is_empty() {
            return;
        }
        crate::notify(cx);
        self._tick = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVED_FOR).await;
            this.update(cx, |this, cx| {
                this.saved.clear();
                crate::notify(cx);
            })
            .ok();
        }));
    }

    /// Take the daemon's view, unless an older one lost the race, keeping every unanswered edit on top.
    fn adopt(&mut self, view: ConfigView, cx: &mut Context<Self>) {
        if self.loaded && view.revision < self.view.revision {
            return;
        }
        let mut settings = view.settings;
        for edit in self.in_flight.iter().chain(&self.pending) {
            settings.apply(edit.clone());
        }
        if !self.loaded || settings.ui.language != self.view.settings.ui.language {
            i18n::set_locale(&settings.ui.language);
        }
        self.view = ConfigView {
            settings,
            locks: view.locks,
            revision: view.revision,
        };
        self.loaded = true;
        crate::notify(cx);
    }
}

/// What a locked row cites as the layer pinning it.
pub fn lock_cause(by: &LockSource) -> String {
    match by {
        LockSource::Managed(path) => i18n::t_with(
            "setting_locked_by_file",
            &[("path", path.display().to_string().into())],
        ),
        LockSource::DropIn(path) => i18n::t_with(
            "setting_locked_by_dropin",
            &[("path", path.display().to_string().into())],
        ),
        LockSource::Env(var) => {
            i18n::t_with("setting_locked_by_env", &[("var", var.clone().into())])
        }
    }
}

fn queue(pending: &mut Vec<SettingsEdit>, edit: SettingsEdit) {
    pending.retain(|queued| !edit.supersedes(queued));
    pending.push(edit);
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

    #[test]
    fn a_drag_queues_one_edit_and_the_last_value_wins() {
        let mut pending = Vec::new();
        for seconds in 1..=12 {
            queue(&mut pending, crossfade(seconds));
        }
        assert_eq!(pending, vec![crossfade(12)]);
    }

    #[test]
    fn edits_to_different_members_both_stay_queued() {
        let mut pending = Vec::new();
        queue(&mut pending, put("a"));
        queue(&mut pending, put("b"));
        queue(&mut pending, crossfade(3));
        assert_eq!(pending, vec![put("a"), put("b"), crossfade(3)]);
    }
}
