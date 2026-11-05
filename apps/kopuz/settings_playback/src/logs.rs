//! The log folder on this machine: open it, bundle it into one file for a bug report, and a debug crash.

use std::path::PathBuf;

use components::format::t;
use components::{icons, style};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{App, IntoElement, ParentElement as _, Styled};
use store::Notices;

const EXPORT_NAME: &str = "kopuz-logs.txt";

pub(crate) fn actions(cx: &App) -> impl IntoElement {
    h_flex()
        .flex_wrap()
        .gap_3()
        .px_5()
        .pt_3()
        .pb_5()
        .child(
            Button::new("open-logs-folder")
                .custom(style::soft_button(cx.theme().foreground, cx))
                .small()
                .icon(icons::icon(icons::FOLDER))
                .label(t("open_logs_folder"))
                .on_click(|_, _, cx| open_folder(cx)),
        )
        .child(
            Button::new("export-logs")
                .custom(style::soft_button(cx.theme().foreground, cx))
                .small()
                .icon(icons::icon(icons::DOWNLOAD))
                .label(t("export_logs"))
                .on_click(|_, _, cx| export(cx)),
        )
        .when(cfg!(debug_assertions), |row| {
            row.child(
                Button::new("trigger-crash")
                    .custom(style::soft_button(cx.theme().danger, cx))
                    .small()
                    .icon(icons::icon(icons::WARNING))
                    // TODO(i18n): a debug-build tool, English-only in the Dioxus app too.
                    .label("Trigger crash (debug)")
                    .on_click(|_, _, _| panic!("manual crash trigger from settings (debug build)")),
            )
        })
}

fn open_folder(cx: &mut App) {
    if let Err(error) = utils::logs::open_log_dir() {
        tracing::warn!(%error, "could not open the logs folder");
        Notices::error(error.to_string(), cx);
    }
}

/// Where the save dialog starts; the log folder itself is a cache nobody browses to.
fn export_dir() -> PathBuf {
    directories::UserDirs::new()
        .map(|dirs| dirs.download_dir().unwrap_or(dirs.home_dir()).to_path_buf())
        .unwrap_or_default()
}

fn export(cx: &mut App) {
    let prompt = cx.prompt_for_new_path(&export_dir(), Some(EXPORT_NAME));
    cx.spawn(async move |cx| {
        let Ok(Ok(Some(dest))) = prompt.await else {
            return;
        };
        let written = cx
            .background_executor()
            .spawn(async move {
                utils::logs::export_logs_named(
                    &dest,
                    &[utils::logs::APP_LATEST, utils::logs::DAEMON_LATEST],
                )
            })
            .await;
        if let Err(error) = written {
            tracing::warn!(%error, "could not export the logs");
            cx.update(|cx| Notices::error(error.to_string(), cx));
        }
    })
    .detach();
}
