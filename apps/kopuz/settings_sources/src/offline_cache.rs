//! The offline cache: whether songs streamed from across the internet are kept, how much of them, and emptying it.

use api::{OfflineCacheStatus, Table};
use components::dialogs::{self, ConfirmKind};
use components::format::{self, t, t_with};
use components::settings::{SettingRow, Toggle, choice, section};
use components::style;
use config::{LibraryEdit, SettingsEdit};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _};
use gpui_kit::{
    Context, IntoElement, ParentElement as _, Render, SharedString, Styled as _, Subscription,
    Task, Window, div,
};
use link::Link;
use store::{ConfigModel, Invalidation, Notices, TablesChanged};

const GB: u64 = 1_000_000_000;
const PRESETS_GB: [u64; 6] = [1, 2, 5, 10, 20, 50];

/// The presets, plus the current cap when a settings file set one that is not among them.
pub fn sizes(current: u64) -> Vec<u64> {
    let mut sizes: Vec<u64> = PRESETS_GB.iter().map(|gb| gb * GB).collect();
    if !sizes.contains(&current) {
        sizes.push(current);
        sizes.sort_unstable();
    }
    sizes
}

pub struct OfflineCache {
    status: Option<OfflineCacheStatus>,
    _fetch: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl OfflineCache {
    pub fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![
            cx.observe(&ConfigModel::global(cx), |_, _, cx| cx.notify()),
            // The daemon marks the tracks changed whenever the cache gains or loses a song.
            cx.subscribe(
                &Invalidation::global(cx),
                |this, _, event: &TablesChanged, cx| {
                    if event.touches(&[Table::Tracks]) {
                        this.refetch(cx);
                    }
                },
            ),
        ];
        let mut this = Self {
            status: None,
            _fetch: None,
            _subscriptions: subscriptions,
        };
        this.refetch(cx);
        this
    }

    fn refetch(&mut self, cx: &mut Context<Self>) {
        let call = Link::global(cx).call(|api| async move { api.offline_cache_status().await });
        self._fetch = Some(cx.spawn(async move |this, cx| {
            let status = call
                .await
                .inspect_err(|error| tracing::warn!(%error, "reading the offline cache failed"))
                .ok();
            this.update(cx, |this, cx| {
                this.status = status;
                cx.notify();
            })
            .ok();
        }));
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        let call = Link::global(cx).call(|api| async move { api.clear_offline_cache().await });
        cx.spawn(async move |this, cx| {
            let result = call.await;
            this.update(cx, |this, cx| {
                if let Err(error) = result {
                    Notices::failed(&error, cx);
                }
                this.refetch(cx);
            })
            .ok();
        })
        .detach();
    }

    fn confirm_clear(&self, used: SharedString, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();
        dialogs::confirm(
            ConfirmKind::Clear,
            t("offline_cache_clear"),
            used,
            move |_, cx| {
                this.update(cx, |this, cx| this.clear(cx)).ok();
            },
            window,
            cx,
        );
    }
}

impl Render for OfflineCache {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let library = ConfigModel::global(cx).read(cx).config().library.clone();
        let max = library.offline_cache_max_bytes;
        let used_bytes = self.status.map(|status| status.used_bytes);
        let used = used_bytes.map(|used| {
            t_with(
                "offline_cache_used",
                &[
                    ("used", format::bytes(used).to_string().into()),
                    ("max", format::bytes(max).to_string().into()),
                ],
            )
        });
        let options = sizes(max)
            .into_iter()
            .map(|bytes| (bytes, format::bytes(bytes)))
            .collect();
        let clear = Button::new("offline-cache-clear")
            .custom(style::tint_text_button(cx.theme().danger, cx))
            .small()
            .label(t("offline_cache_clear"))
            .disabled(used_bytes.is_none_or(|used| used == 0))
            .on_click(cx.listener({
                let used = used.clone().unwrap_or_default();
                move |this, _, window, cx| this.confirm_clear(used.clone(), window, cx)
            }));
        let title = t("offline_cache");
        // Its rows say "offline listening" and "maximum size", so a search for the cache by name keeps them all.
        let named = components::settings::matches(&[title.as_str()], cx);
        let keep = |row: SettingRow| if named { Some(row) } else { row.shown(cx) };
        section(title.clone())
            .children(
                (named || components::settings::query(cx).is_none()).then(|| {
                    div()
                        .px_5()
                        .py_2p5()
                        .text_sm()
                        .text_color(style::white(style::TEXT_SECONDARY, cx))
                        .child(t("offline_cache_description"))
                }),
            )
            .children(keep(
                SettingRow::new(t("offline_cache_enabled"))
                    .bind("offline_cache_enabled", cx)
                    .setting(
                        Toggle::new("offline_cache_enabled")
                            .enabled(library.offline_cache_enabled)
                            .on_change(|on, _, cx| {
                                ConfigModel::edit(
                                    cx,
                                    SettingsEdit::Library(LibraryEdit::OfflineCacheEnabled(*on)),
                                )
                            }),
                    ),
            ))
            .children(keep(
                SettingRow::new(t("offline_cache_max_size"))
                    .bind("offline_cache_max_bytes", cx)
                    .setting(choice(
                        "offline-cache-max",
                        &max,
                        options,
                        ConfigModel::writer(|bytes| {
                            SettingsEdit::Library(LibraryEdit::OfflineCacheMaxBytes(bytes))
                        }),
                    )),
            ))
            .children(keep(
                SettingRow::new(used.unwrap_or_else(|| t("offline_cache"))).control(clear),
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_presets_run_from_one_to_fifty_gigabytes() {
        let presets = sizes(5 * GB);
        assert_eq!(presets, [1, 2, 5, 10, 20, 50].map(|gb| gb * GB));
    }

    #[test]
    fn a_cap_from_a_settings_file_is_offered_in_its_place() {
        assert_eq!(
            sizes(750_000_000),
            [750_000_000, GB, 2 * GB, 5 * GB, 10 * GB, 20 * GB, 50 * GB]
        );
        assert_eq!(sizes(100 * GB).last(), Some(&(100 * GB)));
        assert_eq!(sizes(GB).len(), PRESETS_GB.len());
    }
}
