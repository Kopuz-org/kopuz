//! What Kopuz tells other services, each with a line on what it does and its current answer.

use components::SettingRow;
use components::format::{self, t, t_with};
use components::settings::Toggle;
use config::{IntegrationsEdit, LibraryEdit, PlaybackEdit, SettingsEdit, SystemEdit};
use gpui_kit::component::v_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{AnyElement, App, Context, IntoElement, ParentElement as _, SharedString, Styled};
use store::ConfigModel;

use crate::FirstRun;

fn row(
    key: &'static str,
    title: &'static str,
    description: &'static str,
    on: bool,
    edit: fn(bool) -> SettingsEdit,
    cx: &App,
) -> SettingRow {
    SettingRow::new(t(title))
        .description(t(description))
        .setting(
            Toggle::new(SharedString::from(format!("onboarding-{key}")))
                .enabled(on)
                .on_change(move |on, _, cx| ConfigModel::edit(cx, edit(*on))),
        )
        .bind(key, cx)
}

/// Keeping played songs, said with the cap they are kept up to.
fn offline_cache(settings: &config::Settings, cx: &App) -> SettingRow {
    let size = format::bytes(settings.library.offline_cache_max_bytes);
    SettingRow::new(t_with(
        "onboarding_offline_cache",
        &[("size", size.to_string().into())],
    ))
    .setting(
        Toggle::new("onboarding-offline_cache_enabled")
            .enabled(settings.library.offline_cache_enabled)
            .on_change(|on, _, cx| {
                ConfigModel::edit(
                    cx,
                    SettingsEdit::Library(LibraryEdit::OfflineCacheEnabled(*on)),
                )
            }),
    )
    .bind("offline_cache_enabled", cx)
}

impl FirstRun {
    pub(crate) fn privacy_step(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let settings = ConfigModel::global(cx).read(cx).config().clone();
        // Only a source that keeps a watch history has one to feed.
        let history = self
            .added_source(cx)
            .is_some_and(|source| source.capabilities.watch_history);
        v_flex()
            .w_full()
            .gap_3()
            .child(
                crate::ui::card(cx)
                    .child(row(
                        "discord_presence",
                        "discord_presence",
                        "onboarding_privacy_discord",
                        settings.integrations.discord_presence.unwrap_or(true),
                        |on| {
                            SettingsEdit::Integrations(IntegrationsEdit::DiscordPresence(Some(on)))
                        },
                        cx,
                    ))
                    .child(row(
                        "auto_check_updates",
                        "auto_check_updates",
                        "onboarding_privacy_updates",
                        settings.system.auto_check_updates,
                        |on| SettingsEdit::System(SystemEdit::AutoCheckUpdates(on)),
                        cx,
                    ))
                    .when(history, |card| {
                        card.child(row(
                            "pause_watch_history",
                            "pause_watch_history",
                            "onboarding_privacy_watch_history",
                            settings.playback.pause_watch_history,
                            |on| SettingsEdit::Playback(PlaybackEdit::PauseWatchHistory(on)),
                            cx,
                        ))
                    })
                    .child(offline_cache(&settings, cx)),
            )
            .into_any_element()
    }
}
