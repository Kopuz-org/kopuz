//! Finding songs and fetching them to files.
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
    let mut submitted = use_signal(String::new);
    let found = hooks::downloader::use_search(submitted);

    let fetch = use_callback(move |url: String| {
        failure.set(None);
        active_url.set(url.clone());
        hooks::downloader::start(url, format(), failure);
    });
    let mut do_download = move || {
        let url = url_input().trim().to_string();
        if url.is_empty() {
            return;
        }
        fetch.call(url);
        url_input.set(String::new());
    };
    let mut do_search = move || {
        failure.set(None);
        submitted.set(url_input().trim().to_string());
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
                        "app-icon-button text-white p-2 rounded-lg bg-white/10 transition-colors"
                    } else {
                        "app-icon-button text-slate-400 hover:text-white p-2 rounded-lg hover:bg-white/5 transition-colors"
                    },
                    aria_pressed: *show_opts.read(),
                    title: i18n::t("downloader_options").to_string(),
                    onclick: move |_| show_opts.set(!show_opts()),
                    i { class: "fa-solid fa-sliders" }
                }
            }

            div { class: "flex gap-2 mb-3",
                input {
                    class: "app-search-field flex-1 bg-white/5 border border-white/10 rounded-xl px-4 py-3 text-white placeholder-slate-500 focus:outline-none focus:border-white/30 transition-colors text-sm",
                    placeholder: "{i18n::t(\"downloader_url_placeholder\")}",
                    value: "{url_input}",
                    oninput: move |e| {
                        failure.set(None);
                        url_input.set(e.value());
                    },
                    onkeydown: move |e| {
                        if e.key() == dioxus::prelude::Key::Enter { do_search(); }
                    }
                }
                button {
                    class: "app-icon-button bg-white/5 hover:bg-white/10 text-slate-300 hover:text-white px-4 py-3 rounded-xl transition-colors text-sm shrink-0",
                    title: i18n::t("search").to_string(),
                    onclick: move |_| do_search(),
                    i { class: "fa-solid fa-magnifying-glass" }
                }
                button {
                    class: "app-button-filled bg-white/10 hover:bg-white/20 text-white px-5 py-3 rounded-xl transition-colors font-medium text-sm shrink-0",
                    onclick: move |_| do_download(),
                    i { class: "fa-solid fa-download mr-2" }
                    "{i18n::t(\"downloader_download\")}"
                }
            }

            div { class: "flex gap-2 mb-4 flex-wrap",
                for option in formats.iter().cloned() {
                    button {
                        key: "{option.value}",
                        disabled: option.unavailable.is_some(),
                        title: option.unavailable.as_ref().map(components::forms::text),
                        aria_pressed: *format.read() == option.value,
                        class: if option.unavailable.is_some() {
                            "app-chip text-xs px-3 py-1.5 rounded-lg bg-white/5 text-slate-600 opacity-50 cursor-not-allowed"
                        } else if *format.read() == option.value {
                            "app-chip text-xs px-3 py-1.5 rounded-lg bg-white/20 text-white font-medium transition-colors"
                        } else {
                            "app-chip text-xs px-3 py-1.5 rounded-lg bg-white/5 text-slate-400 hover:text-white hover:bg-white/10 transition-colors"
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

            if !submitted.read().is_empty() {
                SearchResults {
                    query: submitted(),
                    found: found.read().clone(),
                    on_download: fetch,
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
                                class: "app-button-text text-slate-600 hover:text-slate-400 text-xs transition-colors",
                                onclick: move |_| hooks::downloader::clear_history(reload),
                                "{i18n::t(\"downloader_clear_history\")}"
                            }
                        }
                    }
                    if running {
                        ActiveRow { progress, url: active_url() }
                    }
                    for (position, entry) in listed.into_iter().enumerate() {
                        HistoryRow {
                            key: "{position}-{entry.url}",
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

/// What a search or a pasted link turned up, each song with its own download.
/// `found` is `None` while the answer is on its way.
#[component]
fn SearchResults(
    query: String,
    found: Option<Result<Vec<api::DownloadCandidate>, String>>,
    on_download: Callback<String>,
) -> Element {
    match found {
        None => rsx! {
            div { class: "flex justify-center py-6 text-slate-500",
                i { class: "fa-solid fa-spinner fa-spin" }
            }
        },
        Some(Err(error)) => rsx! {
            div { class: "mb-4 rounded-xl border border-red-500/20 bg-red-500/10 px-4 py-3 text-sm text-red-200 whitespace-pre-wrap",
                i { class: "fa-solid fa-triangle-exclamation mr-2 text-red-300" }
                "{error}"
            }
        },
        Some(Ok(ref candidates)) if candidates.is_empty() => rsx! {
            p { class: "text-sm text-slate-500 py-4 text-center",
                "{i18n::t_with(\"no_results_found\", &[(\"query\", query.clone())])}"
            }
        },
        Some(Ok(candidates)) => rsx! {
            div { class: "mb-5 max-h-96 overflow-y-auto rounded-xl border border-white/10 divide-y divide-white/5",
                for (position, candidate) in candidates.into_iter().enumerate() {
                    CandidateRow {
                        key: "{position}-{candidate.url}",
                        candidate,
                        on_download,
                    }
                }
            }
        },
    }
}

#[component]
fn CandidateRow(candidate: api::DownloadCandidate, on_download: Callback<String>) -> Element {
    let byline = [candidate.artist.as_str(), candidate.album.as_str()]
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    let duration = format!(
        "{}:{:02}",
        candidate.duration_secs / 60,
        candidate.duration_secs % 60
    );
    let url = candidate.url.clone();

    rsx! {
        div { class: "flex items-center gap-3 px-3 py-2 hover:bg-white/5 transition-colors",
            if let Some(cover) = candidate.cover_url.as_ref() {
                img {
                    class: "w-10 h-10 rounded object-cover shrink-0 bg-white/5",
                    src: "{cover}",
                    loading: "lazy",
                }
            } else {
                div { class: "w-10 h-10 rounded shrink-0 bg-white/5 flex items-center justify-center",
                    i { class: "fa-solid fa-music text-slate-600 text-xs" }
                }
            }
            div { class: "flex-1 min-w-0",
                p { class: "text-white text-sm truncate", "{candidate.title}" }
                p { class: "text-slate-500 text-xs truncate", "{byline}" }
            }
            if candidate.duration_secs > 0 {
                span { class: "text-slate-500 text-xs tabular-nums shrink-0", "{duration}" }
            }
            button {
                class: "app-icon-button text-slate-400 hover:text-white p-2 rounded-lg hover:bg-white/10 transition-colors shrink-0",
                title: i18n::t("downloader_download").to_string(),
                onclick: move |_| on_download.call(url.clone()),
                i { class: "fa-solid fa-download" }
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
                        div { class: "app-progress-track mt-2 w-full bg-white/10 rounded-full h-1",
                            div {
                                class: if processing {
                                    "h-1 rounded-full bg-yellow-400/60 transition-all duration-300"
                                } else {
                                    "material3-progress h-1 rounded-full bg-white/50 transition-all duration-300"
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
