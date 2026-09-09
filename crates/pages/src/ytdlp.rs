use api::{YtdlpAudioFormat, YtdlpRequest};
use config::AppConfig;
use dioxus::prelude::*;

#[component]
pub fn YtdlpPage(config: Signal<AppConfig>) -> Element {
    let mut url_input = use_signal(String::new);
    let mut format = use_signal(|| YtdlpAudioFormat::BestAudio);
    let mut out_dir = use_signal(|| config.peek().ytdlp_output_dir.clone());
    let mut show_opts = use_signal(|| false);
    let mut failure = use_signal(|| Option::<String>::None);
    // Named before the daemon reports a title, so the row has something to say.
    let mut active_url = use_signal(String::new);
    let progress = hooks::ytdlp::use_progress();
    let history = use_memo(move || config.read().ytdlp_history.clone());

    let mut do_download = move || {
        let url = url_input().trim().to_string();
        if url.is_empty() {
            return;
        }

        failure.set(None);
        active_url.set(url.clone());
        hooks::ytdlp::start(
            YtdlpRequest {
                url,
                output_dir: out_dir(),
                format: format(),
                options: config.peek().ytdlp_options.clone(),
            },
            failure,
        );
        url_input.set(String::new());
    };

    rsx! {
        div { class: "p-6 w-full",

            div { class: "flex items-center justify-between mb-6",
                div {
                    h1 { class: "text-2xl font-bold text-white mb-1",
                        i { class: "fa-solid fa-download mr-3 text-slate-400" }
                        "{i18n::t(\"ytdlp_title\")}"
                    }
                    p { class: "text-slate-500 text-sm", "{i18n::t(\"ytdlp_subtitle\")}" }
                }
                button {
                    class: if *show_opts.read() {
                        "text-white p-2 rounded-lg bg-white/10 transition-colors"
                    } else {
                        "text-slate-400 hover:text-white p-2 rounded-lg hover:bg-white/5 transition-colors"
                    },
                    title: i18n::t("ytdlp_options").to_string(),
                    onclick: move |_| show_opts.set(!show_opts()),
                    i { class: "fa-solid fa-sliders" }
                }
            }

            div { class: "flex gap-2 mb-3",
                input {
                    class: "flex-1 bg-white/5 border border-white/10 rounded-xl px-4 py-3 text-white placeholder-slate-500 focus:outline-none focus:border-white/30 transition-colors text-sm",
                    placeholder: "{i18n::t(\"ytdlp_url_placeholder\")}",
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
                    "{i18n::t(\"ytdlp_download\")}"
                }
            }

            div { class: "flex gap-2 mb-4 flex-wrap",
                for fmt in [YtdlpAudioFormat::BestAudio, YtdlpAudioFormat::Mp3, YtdlpAudioFormat::Flac, YtdlpAudioFormat::Opus, YtdlpAudioFormat::Wav, YtdlpAudioFormat::Video] {
                    button {
                        class: if *format.read() == fmt {
                            "text-xs px-3 py-1.5 rounded-lg bg-white/20 text-white font-medium transition-colors"
                        } else {
                            "text-xs px-3 py-1.5 rounded-lg bg-white/5 text-slate-400 hover:text-white hover:bg-white/10 transition-colors"
                        },
                        onclick: move |_| format.set(fmt),
                        "{format_label(fmt)}"
                    }
                }
            }

            if let Some(error) = failure.read().clone() {
                div { class: "mb-4 rounded-xl border border-red-500/20 bg-red-500/10 px-4 py-3 text-sm text-red-200 whitespace-pre-wrap",
                    i { class: "fa-solid fa-triangle-exclamation mr-2 text-red-300" }
                    "{error}"
                }
            }

            div { class: "flex items-center gap-2 mb-5",
                i { class: "fa-solid fa-folder text-slate-600 text-sm shrink-0" }
                input {
                    class: "flex-1 bg-white/5 border border-white/10 rounded-lg px-3 py-2 text-white text-sm placeholder-slate-600 focus:outline-none focus:border-white/30 transition-colors",
                    placeholder: "{i18n::t(\"ytdlp_output_dir_placeholder\")}",
                    value: "{out_dir}",
                    oninput: move |e| {
                        failure.set(None);
                        out_dir.set(e.value());
                        config.write().ytdlp_output_dir = e.value();
                    }
                }
                button {
                    class: "text-slate-400 hover:text-white transition-colors px-2 py-2 rounded-lg hover:bg-white/5 shrink-0",
                    title: i18n::t("ytdlp_pick_folder").to_string(),
                    onclick: move |_| {
                        spawn(async move {
                            if let Some(folder) = rfd::AsyncFileDialog::new().pick_folder().await {
                                let path = folder.path().to_string_lossy().to_string();
                                out_dir.set(path.clone());
                                config.write().ytdlp_output_dir = path;
                            }
                        });
                    },
                    i { class: "fa-solid fa-folder-open text-sm" }
                }
            }

            if *show_opts.read() {
                OptionsPanel { config }
            }

            if progress.read().running || !history().is_empty() {
                div { class: "space-y-2 mt-2",
                    if !history().is_empty() {
                        div { class: "flex justify-end mb-1",
                            button {
                                class: "text-slate-600 hover:text-slate-400 text-xs transition-colors",
                                onclick: move |_| config.write().ytdlp_history.clear(),
                                "{i18n::t(\"ytdlp_clear_history\")}"
                            }
                        }
                    }
                    if progress.read().running {
                        ActiveRow { progress, url: active_url() }
                    }
                    for entry in history().into_iter() {
                        HistoryRow { entry }
                    }
                }
            } else {
                div { class: "text-center py-16 text-slate-600",
                    i { class: "fa-solid fa-download text-4xl mb-4 block opacity-30" }
                    p { class: "text-sm", "{i18n::t(\"ytdlp_empty_state\")}" }
                }
            }
        }
    }
}

/// The label a format shows in the picker, and the one the daemon stored in
/// history for a finished download.
fn format_label(format: YtdlpAudioFormat) -> String {
    i18n::t(match format {
        YtdlpAudioFormat::BestAudio => "ytdlp_format_best_audio",
        YtdlpAudioFormat::Mp3 => "ytdlp_format_mp3",
        YtdlpAudioFormat::Flac => "ytdlp_format_flac",
        YtdlpAudioFormat::Opus => "ytdlp_format_opus",
        YtdlpAudioFormat::Wav => "ytdlp_format_wav",
        YtdlpAudioFormat::Video => "ytdlp_format_video",
    })
}

fn stored_format_label(stored: &str) -> String {
    format_label(match stored {
        "MP3" => YtdlpAudioFormat::Mp3,
        "FLAC" => YtdlpAudioFormat::Flac,
        "OPUS" => YtdlpAudioFormat::Opus,
        "WAV" => YtdlpAudioFormat::Wav,
        "Video (MP4)" => YtdlpAudioFormat::Video,
        _ => YtdlpAudioFormat::BestAudio,
    })
}

/// A URL is what a row shows until yt-dlp names the file it is writing.
fn shorten(url: &str) -> String {
    url.trim_start_matches("https://")
        .trim_start_matches("http://")
        .chars()
        .take(60)
        .collect()
}

#[component]
fn OptionsPanel(config: Signal<AppConfig>) -> Element {
    let opts = use_memo(move || config.read().ytdlp_options.clone());

    rsx! {
        div { class: "bg-white/5 border border-white/10 rounded-xl p-5 mb-5 space-y-5",

            div {
                p { class: "text-xs font-semibold text-slate-400 mb-3",
                    "{i18n::t(\"ytdlp_section_embed\")}"
                }
                div { class: "grid grid-cols-2 gap-x-6 gap-y-2",
                    OptToggle {
                        label: i18n::t("ytdlp_embed_metadata"),
                        desc: "--embed-metadata",
                        enabled: opts().embed_metadata,
                        on_change: move |v| config.write().ytdlp_options.embed_metadata = v,
                    }
                    OptToggle {
                        label: i18n::t("ytdlp_embed_thumbnail"),
                        desc: "--embed-thumbnail",
                        enabled: opts().embed_thumbnail,
                        on_change: move |v| config.write().ytdlp_options.embed_thumbnail = v,
                    }
                    OptToggle {
                        label: i18n::t("ytdlp_embed_chapters"),
                        desc: "--embed-chapters",
                        enabled: opts().embed_chapters,
                        on_change: move |v| config.write().ytdlp_options.embed_chapters = v,
                    }
                    OptToggle {
                        label: i18n::t("ytdlp_embed_subtitles"),
                        desc: "--embed-subs",
                        enabled: opts().embed_subs,
                        on_change: move |v| config.write().ytdlp_options.embed_subs = v,
                    }
                    OptToggle {
                        label: i18n::t("ytdlp_embed_info_json"),
                        desc: "--embed-info-json",
                        enabled: opts().embed_info_json,
                        on_change: move |v| config.write().ytdlp_options.embed_info_json = v,
                    }
                }
            }

            div { class: "h-px bg-white/5" }

            div {
                p { class: "text-xs font-semibold text-slate-400 mb-3",
                    "{i18n::t(\"ytdlp_section_write\")}"
                }
                div { class: "grid grid-cols-2 gap-x-6 gap-y-2",
                    OptToggle {
                        label: i18n::t("ytdlp_write_thumbnail"),
                        desc: "--write-thumbnail",
                        enabled: opts().write_thumbnail,
                        on_change: move |v| config.write().ytdlp_options.write_thumbnail = v,
                    }
                    OptToggle {
                        label: i18n::t("ytdlp_write_description"),
                        desc: "--write-description",
                        enabled: opts().write_description,
                        on_change: move |v| config.write().ytdlp_options.write_description = v,
                    }
                    OptToggle {
                        label: i18n::t("ytdlp_write_info_json"),
                        desc: "--write-info-json",
                        enabled: opts().write_info_json,
                        on_change: move |v| config.write().ytdlp_options.write_info_json = v,
                    }
                    OptToggle {
                        label: i18n::t("ytdlp_write_subtitles"),
                        desc: "--write-subs",
                        enabled: opts().write_subs,
                        on_change: move |v| config.write().ytdlp_options.write_subs = v,
                    }
                    OptToggle {
                        label: i18n::t("ytdlp_write_auto_subtitles"),
                        desc: "--write-auto-subs",
                        enabled: opts().write_auto_subs,
                        on_change: move |v| config.write().ytdlp_options.write_auto_subs = v,
                    }
                    OptToggle {
                        label: i18n::t("ytdlp_write_comments"),
                        desc: "--write-comments",
                        enabled: opts().write_comments,
                        on_change: move |v| config.write().ytdlp_options.write_comments = v,
                    }
                }
            }

            div { class: "h-px bg-white/5" }

            div {
                p { class: "text-xs font-semibold text-slate-400 mb-3",
                    "{i18n::t(\"ytdlp_section_postprocess\")}"
                }
                div { class: "grid grid-cols-2 gap-x-6 gap-y-2 mb-4",
                    OptToggle {
                        label: i18n::t("ytdlp_remove_sponsors"),
                        desc: "--sponsorblock-remove",
                        enabled: opts().sponsorblock,
                        on_change: move |v| config.write().ytdlp_options.sponsorblock = v,
                    }
                    OptToggle {
                        label: i18n::t("ytdlp_mark_sponsors"),
                        desc: "--sponsorblock-mark",
                        enabled: opts().sponsorblock_mark,
                        on_change: move |v| config.write().ytdlp_options.sponsorblock_mark = v,
                    }
                    OptToggle {
                        label: i18n::t("ytdlp_split_chapters"),
                        desc: "--split-chapters",
                        enabled: opts().split_chapters,
                        on_change: move |v| config.write().ytdlp_options.split_chapters = v,
                    }
                    OptToggle {
                        label: i18n::t("ytdlp_crop_thumbnails"),
                        desc: "--postprocessor-args (square crop)",
                        enabled: opts().postprocess_thumbnail_square,
                        on_change: move |v| config.write().ytdlp_options.postprocess_thumbnail_square = v,
                    }
                }
                div {
                        label { class: "text-xs text-slate-500 mb-1 block",
                            "--convert-thumbnails"
                        }
                        div { class: "flex gap-2",
                        for (val, lbl) in [("", i18n::t("ytdlp_none")), ("jpg", "JPG".to_string()), ("png", "PNG".to_string()), ("webp", "WebP".to_string())] {
                            button {
                                class: if opts().convert_thumbnail == val {
                                    "text-xs px-3 py-1.5 rounded-lg bg-white/20 text-white font-medium transition-colors"
                                } else {
                                    "text-xs px-3 py-1.5 rounded-lg bg-white/5 text-slate-400 hover:text-white hover:bg-white/10 transition-colors"
                                },
                                onclick: move |_| config.write().ytdlp_options.convert_thumbnail = val.to_string(),
                                "{lbl}"
                            }
                        }
                    }
                }
            }

            div { class: "h-px bg-white/5" }

            div {
                p { class: "text-xs font-semibold text-slate-400 mb-3",
                    "{i18n::t(\"ytdlp_section_behavior\")}"
                }
                div { class: "grid grid-cols-2 gap-x-6 gap-y-2 mb-4",
                    OptToggle {
                        label: i18n::t("ytdlp_single_video"),
                        desc: "--no-playlist",
                        enabled: opts().no_playlist,
                        on_change: move |v| config.write().ytdlp_options.no_playlist = v,
                    }
                    OptToggle {
                        label: i18n::t("ytdlp_write_xattrs"),
                        desc: "--xattrs",
                        enabled: opts().xattrs,
                        on_change: move |v| config.write().ytdlp_options.xattrs = v,
                    }
                    OptToggle {
                        label: i18n::t("ytdlp_no_mtime"),
                        desc: "--no-mtime",
                        enabled: opts().no_mtime,
                        on_change: move |v| config.write().ytdlp_options.no_mtime = v,
                    }
                }
                div { class: "grid grid-cols-2 gap-4",
                    div {
                        label { class: "text-xs text-slate-500 mb-1 block",
                            "--limit-rate  (e.g. 1M, 500K)"
                        }
                        input {
                            class: "w-full bg-black/30 border border-white/10 rounded-lg px-3 py-1.5 text-white text-sm placeholder-slate-700 focus:outline-none focus:border-white/30 transition-colors",
                            placeholder: "{i18n::t(\"ytdlp_unlimited\")}",
                            value: "{opts().rate_limit}",
                            oninput: move |e| config.write().ytdlp_options.rate_limit = e.value(),
                        }
                    }
                    div {
                        label { class: "text-xs text-slate-500 mb-1 block",
                            "--cookies-from-browser"
                        }
                        select {
                            class: "w-full bg-black/30 border border-white/10 rounded-lg px-3 py-1.5 text-white text-sm focus:outline-none focus:border-white/30 transition-colors",
                            onchange: move |e| config.write().ytdlp_options.cookies_from_browser = {
                                let v = e.value();
                                if v == "none" { String::new() } else { v }
                            },
                            option { value: "none",     selected: opts().cookies_from_browser.is_empty(), "{i18n::t(\"ytdlp_none\")}" }
                            option { value: "chrome",   selected: opts().cookies_from_browser == "chrome",   "Chrome" }
                            option { value: "firefox",  selected: opts().cookies_from_browser == "firefox",  "Firefox" }
                            option { value: "chromium", selected: opts().cookies_from_browser == "chromium", "Chromium" }
                            option { value: "edge",     selected: opts().cookies_from_browser == "edge",     "Edge" }
                            option { value: "safari",   selected: opts().cookies_from_browser == "safari",   "Safari" }
                            option { value: "brave",    selected: opts().cookies_from_browser == "brave",    "Brave" }
                            option { value: "vivaldi",  selected: opts().cookies_from_browser == "vivaldi",  "Vivaldi" }
                        }
                    }
                    div {
                        label { class: "text-xs text-slate-500 mb-1 flex items-center gap-1.5",
                            span { "--js-runtimes" }
                            i {
                                class: "fa-solid fa-circle-info text-[11px] text-slate-400 cursor-help",
                                title: "{i18n::t(\"ytdlp_js_runtimes_tooltip\")}"
                            }
                        }
                        input {
                            class: "w-full bg-black/30 border border-white/10 rounded-lg px-3 py-1.5 text-white text-sm placeholder-slate-700 focus:outline-none focus:border-white/30 transition-colors",
                            placeholder: "deno, node, bun or quickjs[:/path]",
                            value: "{opts().js_runtimes}",
                            oninput: move |e| config.write().ytdlp_options.js_runtimes = e.value(),
                        }
                    }
                }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct OptToggleProps {
    label: String,
    desc: &'static str,
    enabled: bool,
    on_change: EventHandler<bool>,
}

#[component]
fn OptToggle(props: OptToggleProps) -> Element {
    rsx! {
        button {
            class: "flex items-center gap-2 py-1 text-left group",
            onclick: move |_| props.on_change.call(!props.enabled),
            div {
                class: if props.enabled {
                    "w-4 h-4 rounded border border-white/40 bg-white/20 flex items-center justify-center shrink-0"
                } else {
                    "w-4 h-4 rounded border border-white/15 bg-transparent flex items-center justify-center shrink-0"
                },
                if props.enabled {
                    i { class: "fa-solid fa-check text-white text-[9px]" }
                }
            }
            div {
                p { class: "text-white text-sm leading-none mb-0.5", "{props.label}" }
                p { class: "text-slate-600 text-xs font-mono", "{props.desc}" }
            }
        }
    }
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
        i18n::t("ytdlp_status_processing")
    } else if progress.phase == "downloading" {
        i18n::t_with(
            "ytdlp_status_downloading",
            &[("percent", format!("{percent:.0}"))],
        )
    } else {
        i18n::t("ytdlp_status_waiting")
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

#[derive(Props, Clone, PartialEq)]
struct HistoryRowProps {
    entry: config::YtdlpHistoryEntry,
}

#[component]
fn HistoryRow(props: HistoryRowProps) -> Element {
    let entry = &props.entry;
    let failed = entry.status != "completed";

    let (icon, icon_color) = if failed {
        ("fa-solid fa-circle-xmark", "text-red-400")
    } else {
        ("fa-solid fa-circle-check", "text-green-400")
    };

    let status_text = match (failed, entry.error.clone()) {
        (true, Some(error)) => error,
        (true, None) => entry.status.clone(),
        (false, _) => i18n::t("ytdlp_status_completed"),
    };

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
                        span { class: "text-slate-500 text-xs shrink-0", "{stored_format_label(&entry.format)}" }
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
