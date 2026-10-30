use dioxus::prelude::*;

#[component]
pub fn RipAudio(keys: Vec<String>) -> Element {
    let mut open = use_signal(|| false);
    let mut output = use_signal(String::new);
    let mut pending = use_signal(|| false);
    let mut message = use_signal(String::new);
    let mut job_id = use_signal(|| None::<String>);
    let progress = hooks::jobs::use_job_progress(api::JobKind::AudioRip);
    let busy = pending() || progress.read().running;

    rsx! {
        button {
            class: "inline-flex items-center gap-2 h-9 px-3 rounded-full text-sm border border-white/15 hover:bg-white/10",
            onclick: move |_| open.set(true),
            i { class: "fa-solid fa-compact-disc" }
            "{i18n::t(\"cd_rip_flac\")}"
        }
        if open() {
            div { class: "fixed inset-0 z-50 bg-black/70 flex items-center justify-center p-6",
                div { class: "bg-stone-900 border border-white/15 rounded-xl p-6 w-full max-w-md flex flex-col gap-4",
                    role: "dialog",
                    "aria-modal": "true",
                    "aria-label": i18n::t("cd_rip_flac"),
                    h2 { class: "text-lg font-semibold", "{i18n::t(\"cd_rip_flac\")}" }
                    p { class: "text-sm text-slate-400", "{i18n::t(\"cd_rip_help\")}" }
                    label { class: "text-sm",
                        "{i18n::t(\"cd_output_directory\")}"
                        input {
                            class: "mt-2 w-full rounded bg-black/30 border border-white/15 p-2 text-white",
                            value: "{output}",
                            disabled: busy,
                            oninput: move |event| output.set(event.value()),
                        }
                    }
                    if busy {
                        p { class: "text-sm text-slate-300",
                            "{progress.read().message.clone().unwrap_or_default()}"
                        }
                        if let (Some(current), Some(total)) = (progress.read().current, progress.read().total) {
                            progress { class: "w-full", value: current as f64, max: total.max(1) as f64 }
                        }
                    }
                    if !message.read().is_empty() {
                        p { class: "text-sm text-slate-300", role: "status", "{message}" }
                    }
                    div { class: "flex justify-end gap-3",
                        button {
                            class: "px-3 py-2 rounded hover:bg-white/10",
                            onclick: move |_| open.set(false),
                            "{i18n::t(\"close\")}"
                        }
                        if busy {
                            button {
                                class: "px-3 py-2 rounded border border-white/15",
                                onclick: move |_| {
                                    let id = job_id.read().clone();
                                    let api = hooks::api::consume_api();
                                    spawn(async move {
                                        let id = match id {
                                            Some(id) => Some(id),
                                            None => api.jobs().await.ok().and_then(|jobs| jobs.into_iter().find(|job| job.kind == api::JobKind::AudioRip && job.state == api::JobState::Running).map(|job| job.id)),
                                        };
                                        if let Some(id) = id && let Err(error) = api.cancel_job(id).await {
                                            message.set(error.to_string());
                                        }
                                    });
                                },
                                "{i18n::t(\"cancel\")}"
                            }
                        } else {
                            button {
                                class: "px-3 py-2 rounded bg-white text-black disabled:opacity-40",
                                disabled: output.read().trim().is_empty() || keys.is_empty(),
                                onclick: move |_| {
                                    let api = hooks::api::consume_api();
                                    let keys = keys.clone();
                                    let directory = output.read().trim().to_string();
                                    pending.set(true);
                                    message.set(String::new());
                                    spawn(async move {
                                        let result = api.rip_audio(keys, directory).await;
                                        match result {
                                            Err(error) => message.set(error.to_string()),
                                            Ok(job) => {
                                                job_id.set(Some(job.job_id.clone()));
                                                loop {
                                                    match api.jobs().await {
                                                        Err(error) => { message.set(error.to_string()); break; }
                                                        Ok(jobs) => {
                                                            if let Some(status) = jobs.iter().find(|status| status.id == job.job_id)
                                                                && status.state != api::JobState::Running
                                                            {
                                                                message.set(match &status.error {
                                                                    Some(error) => error.message.clone(),
                                                                    None if status.state == api::JobState::Cancelled => i18n::t("cd_rip_cancelled").to_string(),
                                                                    None => i18n::t("cd_rip_finished").to_string(),
                                                                });
                                                                break;
                                                            }
                                                        }
                                                    }
                                                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                                                }
                                                job_id.set(None);
                                            }
                                        }
                                        pending.set(false);
                                    });
                                },
                                "{i18n::t(\"cd_rip_flac\")}"
                            }
                        }
                    }
                }
            }
        }
    }
}
