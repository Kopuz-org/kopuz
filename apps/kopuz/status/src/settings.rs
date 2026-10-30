//! Settings → Interface → Notifications: what interrupts, and what only lands behind the bell.

use api::JobKind;
use components::format::t;
use components::settings::{section, toggle_row};
use gpui_kit::component::v_flex;
use gpui_kit::{
    App, AppContext as _, Context, IntoElement, ParentElement as _, Render, Styled, Window,
};
use store::LayoutModel;
use store::layout::Section as Group;

/// The kinds a person can silence one at a time.
const KINDS: &[JobKind] = &[
    JobKind::Scan,
    JobKind::LibrarySync,
    JobKind::FavoritesSync,
    JobKind::PlaylistSync,
    JobKind::Download,
    JobKind::UrlDownload,
];

pub fn register(cx: &mut App) {
    settings_interface::register(
        "notifications",
        "interface_notifications",
        4,
        Some(Group::Notifications),
        |window, cx| cx.new(|cx| NotificationSettings::new(window, cx)).into(),
        cx,
    );
}

struct NotificationSettings;

impl NotificationSettings {
    fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&LayoutModel::global(cx), |_, _, cx| cx.notify())
            .detach();
        Self
    }
}

impl Render for NotificationSettings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let notifications = store::layout(cx).notifications.clone();
        let jobs: Vec<_> = KINDS
            .iter()
            .map(|kind| {
                let key = store::kind_key(*kind);
                let on = notifications.toasts(key);
                toggle_row(
                    format!("notifications-job-{key}"),
                    store::kind_label(*kind),
                    on,
                    move |on, cx| {
                        LayoutModel::edit(cx, move |layout| {
                            layout.notifications.jobs.insert(key.to_string(), on);
                        });
                    },
                )
                .description(t("interface_notifications_job_description"))
                .shown(cx)
            })
            .collect();
        v_flex()
            .w_full()
            .child(
                section(t("interface_notifications"))
                    .children(
                        toggle_row(
                            "notifications-errors",
                            t("interface_notifications_errors"),
                            notifications.errors,
                            |on, cx| {
                                LayoutModel::edit(cx, |layout| layout.notifications.errors = on)
                            },
                        )
                        .description(t("interface_notifications_errors_description"))
                        .shown(cx),
                    )
                    .children(jobs.into_iter().flatten()),
            )
            .into_any_element()
    }
}
