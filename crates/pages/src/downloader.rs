//! Fetching a URL to a file.
//!
//! Which tool does it, where it writes and every option it takes are the
//! daemon's; this asks for the formats and the options it publishes, renders
//! them, and sends back what was picked.

use dioxus::prelude::*;

#[component]
pub fn DownloaderPage() -> Element {
    let mut url_input = use_signal(String::new);
    let mut format = use_signal(String::new);
    let mut show_opts = use_signal(|| false);
    let mut failure = use_signal(|| Option::<String>::None);
    // Named before the daemon reports a title, so the row has something to say.
    let mut active_url = use_signal(String::new);
    let reload = use_signal(|| 0u64);
    let progress = hooks::downloader::use_progress();
    let formats = hooks::downloader::use_formats();
    let mut settings = hooks::downloader::use_settings(reload);
    let mut history = hooks::downloader::use_history(reload);

    let formats = formats.read().clone().unwrap_or_default();
    // The first format the daemon offers is the default, so the page does not
    // name one of them itself.
    if format.read().is_empty()
        && let Some(first) = formats.first()
    {
        format.set(first.value.clone());
    }
    let listed = history.read().clone().unwrap_or_default();
    let running = progress.read().running;

    let mut do_download = move || {
        let url = url_input().trim().to_string();
        if url.is_empty() {
            return;
        }
        failure.set(None);
        active_url.set(url.clone());
        hooks::downloader::start(url, format(), failure);
        url_input.set(String::new());
    };

    // The history gains a row when a download finishes, which is the moment
    // the job stops running.
    use_effect(move || {
        if !progress.read().running {
            history.restart();
        }
    });

    rsx! {
        div { class: "p-6 w-full",

            div { class: "flex items-center justify-between mb-6",
                div {
                    h1 { class: "text-2xl font-bold text-white mb-1",
                        i { class: "fa-solid fa-download mr-3 text-slate-400" }
                        "{i18n::t(\"downloader_title\")}"
                    }
                    p { class: "text-slate-500 text-sm", "{i18n::t(\"downloader_subtitle\")}" }
                }
                button {
                    class: if *show_opts.read() {
                        "text-white p-2 rounded-lg bg-white/10 transition-colors"
                    } else {
                        "text-slate-400 hover:text-white p-2 rounded-lg hover:bg-white/5 transition-colors"
                    },
                    title: i18n::t("downloader_options").to_string(),
                    onclick: move |_| show_opts.set(!show_opts()),
                    i { class: "fa-solid fa-sliders" }
                }
            }

            div { class: "flex gap-2 mb-3",
                input {
                    class: "flex-1 bg-white/5 border border-white/10 rounded-xl px-4 py-3 text-white placeholder-slate-500 focus:outline-none focus:border-white/30 transition-colors text-sm",
                    placeholder: "{i18n::t(\"downloader_url_placeholder\")}",
                    value: "{url_input}",
                    oninput: move |e| {
                        failure.set(None);
                        url_input.set(e.value());
                    },
                    onkeydown: move |e| {
                        if e.key() == dioxus::prelude::Key::Enter { do_download(); }
                    }
                }
                button {
                    class: "bg-white/10 hover:bg-white/20 text-white px-5 py-3 rounded-xl transition-colors font-medium text-sm shrink-0",
                    onclick: move |_| do_download(),
                    i { class: "fa-solid fa-download mr-2" }
                    "{i18n::t(\"downloader_download\")}"
                }
            }

            div { class: "flex gap-2 mb-4 flex-wrap",
                for option in formats.iter().cloned() {
                    button {
                        key: "{option.value}",
                        class: if *format.read() == option.value {
                            "text-xs px-3 py-1.5 rounded-lg bg-white/20 text-white font-medium transition-colors"
                        } else {
                            "text-xs px-3 py-1.5 rounded-lg bg-white/5 text-slate-400 hover:text-white hover:bg-white/10 transition-colors"
                        },
                        onclick: {
                            let picked = option.value.clone();
                            move |_| format.set(picked.clone())
                        },
                        "{components::forms::text(&option.label)}"
                    }
                }
            }

            if let Some(error) = failure.read().clone() {
                div { class: "mb-4 rounded-xl border border-red-500/20 bg-red-500/10 px-4 py-3 text-sm text-red-200 whitespace-pre-wrap",
                    i { class: "fa-solid fa-triangle-exclamation mr-2 text-red-300" }
                    "{error}"
                }
            }

            if *show_opts.read() {
                div { class: "bg-white/5 border border-white/10 rounded-xl py-2 mb-5",
                    components::forms::schema_form::SchemaForm {
                        fields: settings.read().clone().unwrap_or_default(),
                        values: Vec::new(),
                        on_change: move |value: api::FieldValue| {
                            hooks::downloader::set_setting(value, reload);
                            settings.restart();
                        },
                    }
                }
            }

            if running || !listed.is_empty() {
                div { class: "space-y-2 mt-2",
                    if !listed.is_empty() {
                        div { class: "flex justify-end mb-1",
                            button {
                                class: "text-slate-600 hover:text-slate-400 text-xs transition-colors",
                                onclick: move |_| hooks::downloader::clear_history(reload),
                                "{i18n::t(\"downloader_clear_history\")}"
                            }
                        }
                    }
                    if running {
                        ActiveRow { progress, url: active_url() }
                    }
                    for entry in listed.into_iter() {
                        HistoryRow {
                            key: "{entry.url}",
                            entry,
                            formats: formats.clone(),
                        }
                    }
                }
            } else {
                div { class: "text-center py-16 text-slate-600",
                    i { class: "fa-solid fa-download text-4xl mb-4 block opacity-30" }
                    p { class: "text-sm", "{i18n::t(\"downloader_empty_state\")}" }
                }
            }
        }
    }
}

/// A URL is what a row shows until the downloader names the file it is writing.
fn shorten(url: &str) -> String {
    url.trim_start_matches("https://")
        .trim_start_matches("http://")
        .chars()
        .take(60)
        .collect()
}

#[component]
fn ActiveRow(progress: Signal<hooks::jobs::JobProgress>, url: String) -> Element {
    let progress = progress.read().clone();
    let processing = progress.phase == "processing";
    let percent = match (progress.current, progress.total) {
        (Some(current), Some(total)) if total > 0 => current as f64 * 100.0 / total as f64,
        _ => 0.0,
    };

    let (icon, icon_color) = if processing {
        ("fa-solid fa-gears", "text-yellow-400")
    } else {
        ("fa-solid fa-spinner fa-spin", "text-blue-400")
    };

    let status_text = if processing {
        i18n::t("downloader_status_processing")
    } else if progress.phase == "downloading" {
        i18n::t_with(
            "downloader_status_downloading",
            &[("percent", format!("{percent:.0}"))],
        )
    } else {
        i18n::t("downloader_status_waiting")
    };

    let title = progress.message.unwrap_or_else(|| shorten(&url));

    rsx! {
        div { class: "bg-white/5 rounded-xl px-4 py-3 border border-white/10",
            div { class: "flex items-start gap-3",
                i { class: "{icon} {icon_color} text-sm mt-0.5 shrink-0" }
                div { class: "flex-1 min-w-0",
                    div { class: "flex items-start justify-between gap-2",
                        span { class: "text-white text-sm truncate flex-1", "{title}" }
                    }
                    p { class: "text-slate-500 text-xs mt-0.5", "{status_text}" }
                    if percent > 0.0 {
                        div { class: "mt-2 w-full bg-white/10 rounded-full h-1",
                            div {
                                class: if processing {
                                    "h-1 rounded-full bg-yellow-400/60 transition-all duration-300"
                                } else {
                                    "h-1 rounded-full bg-white/50 transition-all duration-300"
                                },
                                style: "width: {percent:.1}%"
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn HistoryRow(entry: api::DownloadHistoryEntry, formats: Vec<api::ChoiceOption>) -> Element {
    let failed = entry.state == api::DownloadState::Failed;

    let (icon, icon_color) = if failed {
        ("fa-solid fa-circle-xmark", "text-red-400")
    } else {
        ("fa-solid fa-circle-check", "text-green-400")
    };

    let status_text = match (failed, entry.error.clone()) {
        (true, Some(error)) => error,
        (true, None) => i18n::t("downloader_status_failed"),
        (false, _) => i18n::t("downloader_status_completed"),
    };

    let format = formats
        .iter()
        .find(|option| option.value == entry.format)
        .map(|option| components::forms::text(&option.label))
        .unwrap_or_default();

    let title = if entry.title == entry.url {
        shorten(&entry.url)
    } else {
        entry.title.clone()
    };

    rsx! {
        div { class: "bg-white/5 rounded-xl px-4 py-3 border border-white/10",
            div { class: "flex items-start gap-3",
                i { class: "{icon} {icon_color} text-sm mt-0.5 shrink-0" }
                div { class: "flex-1 min-w-0",
                    div { class: "flex items-start justify-between gap-2",
                        span { class: "text-white text-sm truncate flex-1", "{title}" }
                        span { class: "text-slate-500 text-xs shrink-0", "{format}" }
                    }
                    p {
                        class: if failed {
                            "text-red-400 text-xs mt-0.5 truncate"
                        } else {
                            "text-slate-500 text-xs mt-0.5"
                        },
                        "{status_text}"
                    }
                }
            }
        }
    }
}
