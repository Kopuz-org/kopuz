use components::settings_items::{SettingItem, SettingsSection, ToggleSetting};
use config::AppConfig;
use dioxus::prelude::*;

#[cfg(not(target_os = "android"))]
use crate::theme_editor::ThemeEditorPage;

#[cfg(not(target_os = "android"))]
pub(super) fn theme_editor_section(config: Signal<AppConfig>) -> Element {
    rsx! {
        SettingsSection { title: i18n::t("theme_editor").to_string(),
            div { class: "py-2",
                ThemeEditorPage { config, embedded: true }
            }
        }
    }
}

#[cfg(target_os = "android")]
pub(super) fn theme_editor_section(_config: Signal<AppConfig>) -> Element {
    rsx! {}
}

#[cfg(not(target_os = "android"))]
fn trigger_test_crash() {
    panic!("manual crash trigger from settings (debug build)");
}

#[cfg(not(target_os = "android"))]
pub(super) fn logs_section(mut config: Signal<AppConfig>) -> Element {
    rsx! {
        SettingsSection { title: i18n::t("logs").to_string(),
            div {
                SettingItem {
                    title: i18n::t("enable_tracing").to_string(),
                    control: rsx! {
                        ToggleSetting {
                            enabled: config.read().tracing_enabled,
                            on_change: move |v| {
                                config.write().tracing_enabled = v;
                            },
                        }
                    },
                }
                p {
                    class: "px-5 pb-3 text-xs text-amber-400/80",
                    "{i18n::t(\"tracing_warning\")}"
                }
            }
            div { class: "flex flex-wrap gap-3 px-5 pt-3 pb-5",
                button {
                    r#type: "button",
                    class: "px-4 py-2 rounded-lg bg-white/10 hover:bg-white/20 text-white text-sm transition-colors flex items-center gap-2",
                    onclick: move |_| {
                        if let Err(e) = utils::logs::open_log_dir() {
                            tracing::warn!(error = %e, "failed to open logs folder");
                        }
                    },
                    i { class: "fa-solid fa-folder-open" }
                    "{i18n::t(\"open_logs_folder\")}"
                }
                button {
                    r#type: "button",
                    class: "px-4 py-2 rounded-lg bg-white/10 hover:bg-white/20 text-white text-sm transition-colors flex items-center gap-2",
                    onclick: move |_| {
                        spawn(async move {
                            if let Some(file) = rfd::AsyncFileDialog::new()
                                .set_file_name("kopuz-logs.txt")
                                .save_file()
                                .await
                                && let Err(e) = utils::logs::export_logs(file.path()) {
                                    tracing::warn!(error = %e, "failed to export logs");
                                }
                        });
                    },
                    i { class: "fa-solid fa-file-export" }
                    "{i18n::t(\"export_logs\")}"
                }
                if cfg!(debug_assertions) {
                    button {
                        r#type: "button",
                        class: "px-4 py-2 rounded-lg bg-red-500/20 hover:bg-red-500/30 text-red-300 text-sm transition-colors flex items-center gap-2",
                        onclick: move |_| trigger_test_crash(),
                        i { class: "fa-solid fa-bomb" }
                        "Trigger crash (debug)"
                    }
                }
            }
        }
    }
}

/// Debug-build-only database panel: reset / load release DB / seed / re-run
/// import / vacuum / info, all against the disposable debug DB with a live
/// pool swap (no restart). English-only by design (dev tool).
#[cfg(target_os = "android")]
pub(super) fn logs_section(_config: Signal<AppConfig>) -> Element {
    rsx! {}
}
