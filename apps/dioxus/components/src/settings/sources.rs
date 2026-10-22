//! Source settings controls.

use dioxus::prelude::*;
#[cfg(not(target_os = "android"))]
use rfd::AsyncFileDialog;

#[component]
pub fn MultiDirectoryPicker(
    current_paths: Vec<std::path::PathBuf>,
    on_add: EventHandler<std::path::PathBuf>,
    on_remove: EventHandler<usize>,
) -> Element {
    let add_text = i18n::t("add_folder");
    let remove_text = i18n::t("remove");
    let no_folders_text = i18n::t("no_music_folders");

    rsx! {
        div { class: "flex flex-col gap-2 min-w-0 w-full",
            if current_paths.is_empty() {
                p { class: "text-xs text-slate-500 italic", "{no_folders_text}" }
            }
            for (i, path) in current_paths.iter().enumerate() {
                {
                    let display = path.display().to_string();
                    let row_key = format!("{i}-{display}");
                    rsx! {
                        div { key: "{row_key}",
                            class: "flex items-center justify-between gap-3 bg-white/5 p-2 rounded min-w-0 w-full",
                            span {
                                class: "text-xs text-slate-400 font-mono truncate min-w-0 flex-1",
                                title: "{display}",
                                "{display}"
                            }
                            button {
                                onclick: move |_| {
                                    on_remove.call(i);
                                },
                                class: "text-red-400 hover:text-red-300 text-xs px-2 py-0.5 rounded transition-colors shrink-0",
                                "{remove_text}"
                            }
                        }
                    }
                }
            }
            AddFolderButton { on_add, add_text }
        }
    }
}

#[cfg(not(target_os = "android"))]
#[component]
fn AddFolderButton(on_add: EventHandler<std::path::PathBuf>, add_text: String) -> Element {
    rsx! {
        button {
            onclick: move |_| {
                spawn(async move {
                    if let Some(handle) = AsyncFileDialog::new().pick_folder().await {
                        on_add.call(handle.path().to_path_buf());
                    }
                });
            },
            class: "app-button-tonal bg-white/10 hover:bg-white/20 px-3 py-1 rounded text-sm text-white transition-colors self-start",
            "{add_text}"
        }
    }
}

// Android has no native folder dialog (rfd doesn't work), so request storage permission
// and auto-detect the system Music directory via JNI, falling back to common paths.
#[cfg(target_os = "android")]
#[component]
fn AddFolderButton(on_add: EventHandler<std::path::PathBuf>, add_text: String) -> Element {
    rsx! {
        button {
            onclick: move |_| {
                player::systemint::request_permissions();
                spawn(async move {
                    if !player::systemint::await_media_permission().await {
                        return;
                    }
                    let mut paths = Vec::new();
                    if let Some(android_music) = player::systemint::get_android_music_dir() {
                        paths.push(std::path::PathBuf::from(android_music));
                    }
                    paths.push(std::path::PathBuf::from("/storage/emulated/0/Music"));
                    paths.push(std::path::PathBuf::from("/sdcard/Music"));
                    if let Ok(home) = std::env::var("HOME") {
                        paths.push(std::path::PathBuf::from(home).join("Music"));
                    }
                    for path in paths {
                        if path.exists() {
                            on_add.call(path);
                            break;
                        }
                    }
                });
            },
            class: "app-button-tonal bg-white/10 hover:bg-white/20 px-3 py-1 rounded text-sm text-white transition-colors self-start",
            "{add_text}"
        }
    }
}

#[component]
pub fn SourceSettings(
    /// The configured sources as the daemon reports them, including which one
    /// is active and whether it holds usable credentials.
    sources: Vec<api::SourceInfo>,
    on_add: EventHandler<()>,
    on_delete: EventHandler<String>,
    on_switch: EventHandler<String>,
    on_login: EventHandler<()>,
    /// Folder picker for the active server, when it browses a folder tree. Only
    /// the active server has its creds hydrated, so only it can be browsed.
    remote_folders: Option<crate::settings_remote_folders::RemoteFolderSettings>,
    /// Whether the daemon can start a program on the host.
    host_access: bool,
) -> Element {
    let login_text = i18n::t("login");
    let delete_text = i18n::t("delete");
    let switch_text = i18n::t("switch_to_server");
    let active_text = i18n::t("active_server");

    rsx! {
        div { class: "flex flex-col gap-2 min-w-0 w-full",
            if sources.is_empty() {
                p { class: "text-xs text-white/50 italic", "{i18n::t(\"no_saved_servers\")}" }
            }
            for srv in sources.iter().cloned() {
                {
                    let id = srv.id.clone();
                    let is_active = srv.active;
                    let id_switch = id.clone();
                    let id_delete = id.clone();
                    // A server that browses a folder tree has its own picker for them.
                    let settings: Vec<api::FieldSpec> = srv
                        .settings
                        .iter()
                        .filter(|field| {
                            !(srv.capabilities.browse_folders
                                && matches!(field.kind, api::FieldKind::Directories))
                        })
                        .cloned()
                        .collect();
                    let settings_id = srv.id.clone();
                    let service_name = crate::forms::text(&srv.service.name);
                    let url = srv.detail.clone().unwrap_or_default();
                    // Folders are the whole library definition, so the picker sits on the card.
                    let picker = is_active.then(|| remote_folders.clone()).flatten();
                    let needs_host = srv.capabilities.browser_playback && !host_access;
                    rsx! {
                        div { key: "{srv.id}",
                            class: "app-card flex flex-col gap-2 bg-white/5 p-2 rounded min-w-0 w-full",
                            div { class: "flex flex-col sm:flex-row sm:items-center justify-between gap-2 sm:gap-4 w-full",
                            div { class: "min-w-0 w-full flex-1",
                                div { class: "flex flex-wrap items-center gap-2",
                                    p { class: "min-w-0 text-sm font-medium text-white break-words", "{srv.name}" }
                                    if is_active {
                                        span { class: "app-badge shrink-0 text-[10px] px-2 py-0.5 rounded bg-indigo-500/30 text-indigo-200",
                                            "{active_text}"
                                        }
                                    }
                                }
                                p { class: "text-xs text-white/60", "{i18n::t_with(\"service\", &[(\"name\", service_name.clone().into())])}" }
                                p { class: "text-xs text-white/60 truncate", "{url}" }
                                if is_active && srv.needs_network {
                                    match srv.state {
                                        Some(api::SourceState::Online) => rsx! {
                                            p { class: "text-xs mt-1", style: "color:#3fb950", "{i18n::t(\"connected\")}" }
                                        },
                                        None | Some(api::SourceState::Checking) => rsx! {
                                            p { class: "text-xs mt-1", style: "color:#d8a23a", "{i18n::t(\"connecting\")}" }
                                        },
                                        // Only a session the service called signed out
                                        // is one signing in again can fix.
                                        Some(api::SourceState::AuthExpired) => rsx! {
                                            div { class: "flex items-center gap-2 mt-1",
                                                p { class: "text-xs", style: "color:#e5534b", "{i18n::t(\"source_signed_out\")}" }
                                                button {
                                                    onclick: move |_| on_login.call(()),
                                                    class: "app-button-tonal text-xs bg-white/10 hover:bg-white/20 px-2 py-0.5 rounded text-white transition-colors",
                                                    "{login_text}"
                                                }
                                            }
                                        },
                                        Some(api::SourceState::Offline) => rsx! {
                                            p { class: "text-xs mt-1", style: "color:#e5534b", "{i18n::t(\"source_not_responding\")}" }
                                        },
                                    }
                                }
                            }
                            div { class: "flex flex-wrap items-center gap-2 shrink-0",
                                if !is_active {
                                    button {
                                        onclick: move |_| on_switch.call(id_switch.clone()),
                                        class: "app-button-tonal text-xs bg-white/10 hover:bg-white/20 px-2 py-1 rounded text-white transition-colors",
                                        "{switch_text}"
                                    }
                                }
                                if !srv.permanent {
                                    button {
                                        onclick: move |_| on_delete.call(id_delete.clone()),
                                        class: "app-button-text app-button-danger text-red-400 hover:text-red-300 text-sm px-2 py-1 transition-colors",
                                        "{delete_text}"
                                    }
                                }
                            }
                            }
                            if let Some(picker) = picker {
                                div { class: "flex flex-col gap-2 border-t border-white/10 pt-2",
                                    p { class: "text-xs text-white/60", "{i18n::t(\"remote_music_folders\")}" }
                                    crate::settings_remote_folders::RemoteFolderPicker { settings: picker }
                                }
                            }
                            if srv.capabilities.accounts && srv.authenticated {
                                AccountPicker { source_id: srv.id.clone() }
                            }
                            if needs_host {
                                crate::settings_popups::HostAccessWarning {
                                    message: i18n::t("browser_playback_needs_host").to_string(),
                                }
                            }
                            if is_active && srv.needs_network {
                                ConnectionTest { source_id: srv.id.clone() }
                            }
                            if is_active && srv.authenticated && srv.capabilities.storage_quota {
                                CloudStorage { source_id: srv.id.clone(), uploads: srv.capabilities.uploads }
                            }
                            if !settings.is_empty() {
                                div { class: "border-t border-white/10 pt-2",
                                    crate::forms::schema_form::SchemaForm {
                                        fields: settings.clone(),
                                        values: Vec::new(),
                                        on_change: move |value: api::FieldValue| {
                                            hooks::sources::set_source_settings(
                                                settings_id.clone(),
                                                vec![value],
                                            );
                                        },
                                    }
                                }
                            }
                        }
                    }
                }
            }
            button {
                onclick: move |_| on_add.call(()),
                class: "app-button-tonal bg-white/10 hover:bg-white/20 px-3 py-1 rounded text-sm text-white transition-colors self-start",
                "{i18n::t(\"add_source\")}"
            }
        }
    }
}

#[component]
fn CloudStorage(source_id: String, uploads: bool) -> Element {
    let api = hooks::api::use_api();
    let quota_api = api.clone();
    let quota_id = source_id.clone();
    let mut quota = use_resource(move || {
        let api = quota_api.clone();
        let id = quota_id.clone();
        async move { api.storage_quota(id).await }
    });
    let mut uploading = use_signal(|| false);
    let mut current_file = use_signal(String::new);
    let mut picker_version = use_signal(|| 0_u64);
    let quota_value = quota.read().clone();
    rsx! {
        div { class: "flex flex-col gap-2 border-t border-white/10 pt-2",
            match quota_value {
                Some(Ok(value)) => {
                    let used = format!("{:.1} MiB", value.used_bytes as f64 / 1_048_576.0);
                    let total = format!("{:.1} MiB", value.quota_bytes as f64 / 1_048_576.0);
                    rsx! { p { class: "text-xs text-white/60", "{i18n::t_with(\"cloud_storage_usage\", &[(\"used\", used.clone().into()), (\"total\", total.clone().into())])}" } }
                }
                Some(Err(error)) => rsx! { p { class: "text-xs text-red-400", "{crate::forms::error_text(&error)}" } },
                None => rsx! { p { class: "text-xs text-white/60", "{i18n::t(\"cloud_storage_loading\")}" } },
            }
            if uploads {
                label { class: "app-button-tonal inline-flex items-center justify-center self-start px-3 py-1 rounded text-sm text-white bg-white/10 hover:bg-white/20 transition-colors cursor-pointer",
                    "{i18n::t(\"upload_music\")}"
                    input {
                        key: "{picker_version}",
                        class: "hidden",
                        r#type: "file",
                        accept: ".mp3,.m4a,.flac,.wav,.ogg,.opus,.wma,.aiff",
                        multiple: true,
                        disabled: uploading(),
                        onchange: move |event| {
                            if uploading() { return; }
                            let files = event.files();
                            if files.is_empty() { return; }
                            let api = api.clone();
                            let id = source_id.clone();
                            uploading.set(true);
                            spawn(async move {
                                let mut uploaded = 0;
                                for file in files {
                                    let filename = file.name();
                                    current_file.set(filename.clone());
                                    if file.size() > api::MAX_MUSIC_UPLOAD_BYTES as u64 {
                                        hooks::toast::toast_error(&i18n::t("music_upload_too_large"));
                                        break;
                                    }
                                    let content = match file.read_bytes().await {
                                        Ok(bytes) => bytes.to_vec(),
                                        Err(error) => {
                                            hooks::toast::toast_error(&error.to_string());
                                            break;
                                        }
                                    };
                                    let upload = api::TrackUpload { source_id: id.clone(), filename, content };
                                    if let Err(error) = api.upload_track(upload).await {
                                        hooks::toast::toast_api_error(&error);
                                        break;
                                    }
                                    uploaded += 1;
                                }
                                if uploaded > 0 {
                                    hooks::toast::toast(&i18n::t_with("music_uploaded", &[("count", uploaded.into())]));
                                    match api.sources().await {
                                        Ok(sources) if sources.iter().any(|source| source.id == id && source.active) => {
                                            if let Err(error) = api.start_job(api::JobKind::LibrarySync).await {
                                                hooks::toast::toast_api_error(&error);
                                            }
                                        }
                                        Err(error) => hooks::toast::toast_api_error(&error),
                                        _ => {},
                                    }
                                }
                                quota.restart();
                                uploading.set(false);
                                current_file.set(String::new());
                                picker_version += 1;
                            });
                        },
                    }
                }
            }
            if uploading() {
                p { class: "text-xs text-white/60", "{i18n::t_with(\"uploading_music\", &[(\"name\", current_file().into())])}" }
            }
        }
    }
}

#[component]
fn AccountPicker(source_id: String) -> Element {
    let api = hooks::use_api();
    let mut chosen = use_signal(|| None::<Option<String>>);
    let listed_id = source_id.clone();
    let accounts = use_resource(move || {
        let api = api.clone();
        let id = listed_id.clone();
        async move {
            api.accounts(id)
                .await
                .map_err(|error| crate::forms::error_text(&error))
        }
    });

    let accounts = match &*accounts.read() {
        Some(Ok(accounts)) if accounts.len() > 1 => accounts.clone(),
        Some(Err(error)) => {
            let message =
                i18n::t_with("source_accounts_failed", &[("error", error.clone().into())]);
            return rsx! {
                p { class: "text-xs text-red-400 border-t border-white/10 pt-2", "{message}" }
            };
        }
        _ => return rsx! {},
    };
    let active = chosen().unwrap_or_else(|| {
        accounts
            .iter()
            .find(|account| account.active)
            .and_then(|account| account.id.clone())
    });
    let options: Vec<(String, String)> = accounts
        .iter()
        .map(|account| {
            let label = match &account.handle {
                Some(handle) => format!("{} ({handle})", account.name),
                None => account.name.clone(),
            };
            (account.id.clone().unwrap_or_default(), label)
        })
        .collect();
    let current = active.clone();

    rsx! {
        div { class: "border-t border-white/10 pt-2",
            crate::settings_items::SettingItem {
                title: i18n::t("source_account"),
                control: rsx! {
                    crate::settings_items::AppSelect {
                        value: active.clone().unwrap_or_default(),
                        options,
                        on_change: move |picked: String| {
                            let account = (!picked.is_empty()).then_some(picked);
                            if account == current {
                                return;
                            }
                            chosen.set(Some(account.clone()));
                            let api = hooks::consume_api();
                            let id = source_id.clone();
                            spawn(async move {
                                if let Err(error) = api.switch_account(id, account).await {
                                    tracing::warn!(%error, "switching the source account failed");
                                    chosen.set(None);
                                    hooks::toast::toast_api_error(&error);
                                }
                            });
                        },
                    }
                },
            }
            p { class: "text-xs text-white/50 px-5 pb-2 -mt-1", "{i18n::t(\"source_account_help\")}" }
        }
    }
}

/// Runs the active source's connection test and lists each step, so a bad
/// day says which part broke and the report can go in a bug report as is.
#[component]
fn ConnectionTest(source_id: String) -> Element {
    let mut running = use_signal(|| false);
    let mut report = use_signal(|| None::<Result<Vec<api::ConnectionCheck>, String>>);

    let run = move |_| {
        let id = source_id.clone();
        report.set(None);
        running.set(true);
        spawn(async move {
            let result = hooks::sources::test_connection(id)
                .await
                .map_err(|error| crate::forms::error_text(&error));
            report.set(Some(result));
            running.set(false);
        });
    };

    rsx! {
        div { class: "flex flex-col gap-2 border-t border-white/10 pt-2",
            div { class: "flex flex-wrap items-center gap-2",
                button {
                    onclick: run,
                    disabled: running(),
                    class: "app-button-tonal text-xs bg-white/10 hover:bg-white/20 disabled:opacity-60 px-2 py-1 rounded text-white transition-colors",
                    if running() {
                        "{i18n::t(\"connection_testing\")}"
                    } else {
                        "{i18n::t(\"connection_test\")}"
                    }
                }
                if let Some(Ok(checks)) = report() {
                    button {
                        onclick: move |_| {
                            crate::common::shared::copy_to_clipboard(&report_text(&checks));
                            hooks::toast::toast(&i18n::t("connection_report_copied"));
                        },
                        class: "app-button-text text-xs text-white/70 hover:text-white px-2 py-1 transition-colors",
                        "{i18n::t(\"connection_copy_report\")}"
                    }
                }
            }
            div { role: "status", class: "contents",
                match report() {
                    Some(Ok(checks)) => rsx! {
                        ul { class: "flex flex-col gap-1",
                            for check in checks {
                                li { class: "flex items-start gap-2 text-xs",
                                    i {
                                        class: match check.outcome {
                                            api::CheckOutcome::Passed => "fa-solid fa-circle-check mt-0.5 text-emerald-400",
                                            api::CheckOutcome::Failed => "fa-solid fa-circle-xmark mt-0.5 text-rose-400",
                                            api::CheckOutcome::Skipped => "fa-solid fa-circle-minus mt-0.5 text-white/40",
                                        },
                                        aria_hidden: "true",
                                    }
                                    div { class: "min-w-0",
                                        p { class: "text-white/90",
                                            "{step_label(check.step)}"
                                            span { class: "sr-only", " {outcome_label(check.outcome)}" }
                                        }
                                        p { class: "text-white/50 break-words", "{check.detail}" }
                                    }
                                }
                            }
                        }
                    },
                    Some(Err(error)) => rsx! {
                        p { class: "text-xs text-rose-300 break-words",
                            "{i18n::t_with(\"connection_test_failed\", &[(\"error\", error.clone().into())])}"
                        }
                    },
                    None => rsx! {},
                }
            }
        }
    }
}

fn step_label(step: api::CheckStep) -> String {
    match step {
        api::CheckStep::Reach => i18n::t("connection_step_reach"),
        api::CheckStep::Session => i18n::t("connection_step_session"),
        api::CheckStep::SignedInPlayback => i18n::t("connection_step_signed_in_playback"),
        api::CheckStep::AnonymousPlayback => i18n::t("connection_step_anonymous_playback"),
        api::CheckStep::PlaybackToken => i18n::t("connection_step_playback_token"),
    }
}

fn outcome_label(outcome: api::CheckOutcome) -> String {
    match outcome {
        api::CheckOutcome::Passed => i18n::t("connection_step_passed"),
        api::CheckOutcome::Failed => i18n::t("connection_step_failed"),
        api::CheckOutcome::Skipped => i18n::t("connection_step_skipped"),
    }
}

/// English and in step order whatever the UI language, since it is read by
/// whoever answers the bug report.
fn report_text(checks: &[api::ConnectionCheck]) -> String {
    let mut text = utils::build_info::summary();
    for check in checks {
        text.push_str(&format!(
            "\n{:?}: {:?}: {}",
            check.step, check.outcome, check.detail
        ));
    }
    text
}
