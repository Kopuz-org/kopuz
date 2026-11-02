//! How the player behaves: what Back does partway through a song, and whether played songs are kept for offline, up to how much.

use components::format::{self, t};
use components::settings::Toggle;
use components::{Choice, SettingRow};
use config::{BackBehavior, LibraryEdit, PlaybackEdit, SettingsEdit};
use gpui_kit::component::v_flex;
use gpui_kit::{AnyElement, App, Context, IntoElement, ParentElement as _, Styled};
use settings_sources::offline_cache::sizes;
use store::ConfigModel;

use crate::FirstRun;

/// Keeping played songs, and the cap on them once they are kept.
fn offline_cache(settings: &config::Settings, cx: &App) -> Vec<SettingRow> {
    let library = &settings.library;
    let keep = SettingRow::new(t("offline_cache_enabled"))
        .setting(
            Toggle::new("onboarding-offline_cache_enabled")
                .enabled(library.offline_cache_enabled)
                .on_change(|on, _, cx| {
                    ConfigModel::edit(
                        cx,
                        SettingsEdit::Library(LibraryEdit::OfflineCacheEnabled(*on)),
                    )
                }),
        )
        .bind("offline_cache_enabled", cx);
    let max = library.offline_cache_max_bytes;
    let options = sizes(max)
        .into_iter()
        .map(|bytes| (bytes, format::bytes(bytes)))
        .collect();
    let cap = SettingRow::new(t("offline_cache_max_size"))
        .setting(
            Choice::new("onboarding-offline_cache_max", max, options).on_change(
                ConfigModel::writer(|bytes| {
                    SettingsEdit::Library(LibraryEdit::OfflineCacheMaxBytes(bytes))
                }),
            ),
        )
        .bind("offline_cache_max_bytes", cx);
    let mut rows = vec![keep];
    // A cap on a cache that keeps nothing would only be a question with no effect.
    rows.extend(library.offline_cache_enabled.then_some(cap));
    rows
}

impl FirstRun {
    pub(crate) fn playback_step(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let settings = ConfigModel::global(cx).read(cx).config().clone();
        let back = Choice::new(
            "onboarding-back_behavior",
            settings.playback.back_behavior,
            vec![
                (BackBehavior::RewindThenPrev, t("back_behavior_rewind")),
                (BackBehavior::AlwaysPrev, t("back_behavior_always_prev")),
            ],
        )
        .on_change(ConfigModel::writer(|value| {
            SettingsEdit::Playback(PlaybackEdit::BackBehavior(value))
        }));
        v_flex()
            .w_full()
            .child(
                crate::ui::card(cx)
                    .child(
                        SettingRow::new(t("back_behavior"))
                            .description(t("back_behavior_description"))
                            .setting(back)
                            .bind("back_behavior", cx),
                    )
                    .children(offline_cache(&settings, cx)),
            )
            .into_any_element()
    }
}
