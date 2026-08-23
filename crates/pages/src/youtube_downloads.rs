use crate::youtube_download_jobs::{
    AudioFormat, DownloadJob, JOBS, JobStatus, clear_finished_jobs, run_preflight_checks,
    seed_from_history, start_download,
};
use config::AppConfig;
use dioxus::prelude::*;
use reader::Track;
use server::youtube_download::YoutubeDownloadClient;

#[component]
pub fn YoutubeDownloadsPage(config: Signal<AppConfig>) -> Element {
    let mut query = use_signal(String::new);
    let mut results = use_signal(Vec::<Track>::new);
    let mut searching = use_signal(|| false);
    let mut searched = use_signal(|| false);
    let mut page_error = use_signal(|| Option::<String>::None);
    let mut format = use_signal(|| AudioFormat::Original);
    let mut out_dir = use_signal(|| initial_output_directory(&config.peek()));
    let mut show_options = use_signal(|| false);
    let mut resolving_link = use_signal(|| false);

    use_hook(move || {
        seed_from_history(&config.peek().youtube_download_history);
    });

    // Searching by name is the main path; a link in the box is the exception,
    // so the same field detects one instead of asking the user which mode they
    // are in. Nothing but a real YouTube track link parses, so a search phrase
    // can never be mistaken for one.
    let pasted_link = use_memo(move || server::youtube_download::parse_video_id(&query()));

    let mut search = move || {
        let search_query = query().trim().to_string();
        if search_query.is_empty() || *searching.peek() {
            return;
        }
        searching.set(true);
        searched.set(true);
        page_error.set(None);
        results.set(Vec::new());
        let client = YoutubeDownloadClient::from_config(&config.peek());
        spawn(async move {
            match client.search(&search_query).await {
                Ok(found) => results.set(found.into_iter().take(30).collect()),
                Err(error) => page_error.set(Some(i18n::t_with(
                    "youtube_download_error_search",
                    &[("error", error)],
                ))),
            }
            searching.set(false);
        });
    };

    let mut download_link = move || {
        if pasted_link().is_none() || *resolving_link.peek() {
            return;
        }
        let link = query().trim().to_string();
        resolving_link.set(true);
        page_error.set(None);
        let client = YoutubeDownloadClient::from_config(&config.peek());
        spawn(async move {
            match client.track_from_link(&link).await {
                Ok(track) => {
                    if queue_download(track, config, out_dir, format, page_error) {
                        query.set(String::new());
                    }
                }
                Err(error) => page_error.set(Some(i18n::t_with(
                    "youtube_download_error_link",
                    &[("error", error)],
                ))),
            }
            resolving_link.set(false);
        });
    };

    let is_vaxry = matches!(config.read().ui_style, config::UiStyle::Vaxry);

    rsx! {
        div {
            class: if is_vaxry { "p-6 w-full" } else { "p-8 w-full" },

            div { class: "flex items-center justify-between mb-6",
                div {
                    h1 { class: "text-2xl font-bold text-white mb-1",
                        i { class: "fa-brands fa-youtube mr-3 text-red-400" }
                        "{i18n::t(\"youtube_download_title\")}"
                    }
                    p { class: "text-slate-500 text-sm",
                        "{i18n::t(\"youtube_download_subtitle\")}"
                    }
                }
                button {
                    class: if *show_options.read() {
                        "text-white p-2 rounded-lg bg-white/10 transition-colors"
                    } else {
                        "text-slate-400 hover:text-white p-2 rounded-lg hover:bg-white/5 transition-colors"
                    },
                    title: i18n::t("youtube_download_options"),
                    onclick: move |_| show_options.set(!show_options()),
                    i { class: "fa-solid fa-sliders" }
                }
            }

            div { class: "flex gap-2 mb-3",
                input {
                    class: "flex-1 bg-white/5 border border-white/10 rounded-xl px-4 py-3 text-white placeholder-slate-500 focus:outline-none focus:border-white/30 transition-colors text-sm",
                    placeholder: "{i18n::t(\"youtube_download_search_placeholder\")}",
                    value: "{query}",
                    oninput: move |event| {
                        page_error.set(None);
                        query.set(event.value());
                    },
                    onkeydown: move |event| {
                        event.stop_propagation();
                        if event.key() == Key::Enter {
                            if pasted_link().is_some() {
                                download_link();
                            } else {
                                search();
                            }
                        }
                    }
                }
                button {
                    class: "bg-white/10 hover:bg-white/20 disabled:opacity-50 disabled:cursor-wait text-white px-5 py-3 rounded-xl transition-colors font-medium text-sm shrink-0",
                    disabled: *searching.read() || *resolving_link.read(),
                    onclick: move |_| {
                        if pasted_link().is_some() {
                            download_link();
                        } else {
                            search();
                        }
                    },
                    if *resolving_link.read() {
                        i { class: "fa-solid fa-spinner fa-spin mr-2" }
                        "{i18n::t(\"youtube_download_resolving_link\")}"
                    } else if *searching.read() {
                        i { class: "fa-solid fa-spinner fa-spin mr-2" }
                        "{i18n::t(\"youtube_download_searching\")}"
                    } else if pasted_link().is_some() {
                        i { class: "fa-solid fa-download mr-2" }
                        "{i18n::t(\"youtube_download_download_link\")}"
                    } else {
                        i { class: "fa-solid fa-magnifying-glass mr-2" }
                        "{i18n::t(\"youtube_download_search\")}"
                    }
                }
            }

            if pasted_link().is_some() {
                p { class: "text-xs text-slate-500 mb-3 flex items-center gap-1.5",
                    i { class: "fa-solid fa-link text-[10px]" }
                    "{i18n::t(\"youtube_download_link_detected\")}"
                }
            }

            div { class: "flex gap-2 mb-4 flex-wrap",
                for candidate in [
                    AudioFormat::Original,
                    AudioFormat::Mp3,
                    AudioFormat::Flac,
                    AudioFormat::Opus,
                    AudioFormat::Wav,
                ] {
                    button {
                        class: if *format.read() == candidate {
                            "text-xs px-3 py-1.5 rounded-lg bg-white/20 text-white font-medium transition-colors"
                        } else {
                            "text-xs px-3 py-1.5 rounded-lg bg-white/5 text-slate-400 hover:text-white hover:bg-white/10 transition-colors"
                        },
                        onclick: move |_| {
                            page_error.set(None);
                            format.set(candidate);
                        },
                        "{candidate.label()}"
                    }
                }
            }

            div { class: "flex items-center gap-2 mb-5",
                i { class: "fa-solid fa-folder text-slate-600 text-sm shrink-0" }
                input {
                    class: "flex-1 bg-white/5 border border-white/10 rounded-lg px-3 py-2 text-white text-sm placeholder-slate-600 focus:outline-none focus:border-white/30 transition-colors",
                    placeholder: "{i18n::t(\"youtube_download_output_dir_placeholder\")}",
                    value: "{out_dir}",
                    oninput: move |event| {
                        page_error.set(None);
                        let value = event.value();
                        out_dir.set(value.clone());
                        config.write().youtube_download_output_dir = value;
                    },
                    onkeydown: move |event| event.stop_propagation(),
                }
                button {
                    class: "text-slate-400 hover:text-white transition-colors px-2 py-2 rounded-lg hover:bg-white/5 shrink-0",
                    title: i18n::t("youtube_download_pick_folder"),
                    onclick: move |_| {
                        spawn(async move {
                            if let Some(folder) = rfd::AsyncFileDialog::new().pick_folder().await {
                                let path = folder.path().to_string_lossy().to_string();
                                out_dir.set(path.clone());
                                config.write().youtube_download_output_dir = path;
                            }
                        });
                    },
                    i { class: "fa-solid fa-folder-open text-sm" }
                }
            }

            if *show_options.read() {
                OptionsPanel { config }
            }

            if let Some(error) = page_error.read().clone() {
                div { class: "mb-5 rounded-xl border border-red-500/20 bg-red-500/10 px-4 py-3 text-sm text-red-200 whitespace-pre-wrap",
                    i { class: "fa-solid fa-triangle-exclamation mr-2 text-red-300" }
                    "{error}"
                }
            }

            if !results.read().is_empty() {
                div { class: "mb-7",
                    p { class: "text-xs font-semibold uppercase tracking-wider text-slate-500 mb-2",
                        "{i18n::t(\"youtube_download_results\")}"
                    }
                    div { class: "space-y-2",
                        for track in results.read().clone() {
                            SearchResultRow {
                                track,
                                on_download: move |track: Track| {
                                    queue_download(track, config, out_dir, format, page_error);
                                }
                            }
                        }
                    }
                }
            } else if *searched.read() && !*searching.read() && page_error.read().is_none() {
                div { class: "text-center py-10 text-slate-600",
                    i { class: "fa-solid fa-magnifying-glass text-3xl mb-3 block opacity-30" }
                    p { class: "text-sm", "{i18n::t(\"youtube_download_no_results\")}" }
                }
            } else if !*searched.read() {
                div { class: "text-center py-10 text-slate-600",
                    i { class: "fa-brands fa-youtube text-4xl mb-4 block opacity-30" }
                    p { class: "text-sm", "{i18n::t(\"youtube_download_empty_state\")}" }
                }
            }

            if !JOBS.read().is_empty() {
                div { class: "space-y-2 mt-2",
                    div { class: "flex items-center justify-between mb-1",
                        p { class: "text-xs font-semibold uppercase tracking-wider text-slate-500",
                            "{i18n::t(\"youtube_download_jobs\")}"
                        }
                        button {
                            class: "text-slate-600 hover:text-slate-400 text-xs transition-colors",
                            onclick: move |_| {
                                clear_finished_jobs();
                                config.write().youtube_download_history.clear();
                            },
                            "{i18n::t(\"youtube_download_clear_history\")}"
                        }
                    }
                    for job in JOBS.read().clone() {
                        JobRow { job }
                    }
                }
            }
        }
    }
}

/// Preflight and hand a track to the job driver. Shared by the search results
/// and the pasted-link path so both enforce the same checks. Returns whether the
/// download was actually queued.
fn queue_download(
    track: Track,
    config: Signal<AppConfig>,
    out_dir: Signal<String>,
    format: Signal<AudioFormat>,
    mut page_error: Signal<Option<String>>,
) -> bool {
    let options = config.peek().youtube_download_options.clone();
    let video_id = track.id.key().into_owned();
    if let Err(error) = run_preflight_checks(&video_id, &out_dir(), format(), &options) {
        page_error.set(Some(error));
        return false;
    }
    let client = YoutubeDownloadClient::from_config(&config.peek());
    start_download(track, client, out_dir(), format(), options);
    true
}

fn initial_output_directory(config: &AppConfig) -> String {
    if !config.youtube_download_output_dir.trim().is_empty() {
        return config.youtube_download_output_dir.clone();
    }
    directories::UserDirs::new()
        .and_then(|directories| directories.audio_dir().map(|path| path.to_path_buf()))
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[component]
fn OptionsPanel(config: Signal<AppConfig>) -> Element {
    let options = use_memo(move || config.read().youtube_download_options.clone());

    rsx! {
        div { class: "bg-white/5 border border-white/10 rounded-xl p-5 mb-5",
            p { class: "text-xs font-semibold text-slate-400 mb-3",
                "{i18n::t(\"youtube_download_file_options\")}"
            }
            div { class: "grid grid-cols-1 md:grid-cols-2 gap-x-6 gap-y-2",
                OptionToggle {
                    label: i18n::t("youtube_download_embed_metadata"),
                    description: i18n::t("youtube_download_embed_metadata_desc"),
                    enabled: options().embed_metadata,
                    on_change: move |value| config.write().youtube_download_options.embed_metadata = value,
                }
                OptionToggle {
                    label: i18n::t("youtube_download_embed_thumbnail"),
                    description: i18n::t("youtube_download_embed_thumbnail_desc"),
                    enabled: options().embed_thumbnail,
                    on_change: move |value| config.write().youtube_download_options.embed_thumbnail = value,
                }
                OptionToggle {
                    label: i18n::t("youtube_download_write_thumbnail"),
                    description: i18n::t("youtube_download_write_thumbnail_desc"),
                    enabled: options().write_thumbnail,
                    on_change: move |value| config.write().youtube_download_options.write_thumbnail = value,
                }
                OptionToggle {
                    label: i18n::t("youtube_download_organize_album"),
                    description: i18n::t("youtube_download_organize_album_desc"),
                    enabled: options().organize_by_album,
                    on_change: move |value| config.write().youtube_download_options.organize_by_album = value,
                }
                OptionToggle {
                    label: i18n::t("youtube_download_overwrite"),
                    description: i18n::t("youtube_download_overwrite_desc"),
                    enabled: options().overwrite_existing,
                    on_change: move |value| config.write().youtube_download_options.overwrite_existing = value,
                }
            }
            p { class: "text-[11px] text-slate-600 mt-4",
                i { class: "fa-solid fa-circle-info mr-1.5" }
                "{i18n::t(\"youtube_download_ffmpeg_hint\")}"
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct OptionToggleProps {
    label: String,
    description: String,
    enabled: bool,
    on_change: EventHandler<bool>,
}

#[component]
fn OptionToggle(props: OptionToggleProps) -> Element {
    rsx! {
        button {
            class: "flex items-start gap-2 py-1.5 text-left group",
            onclick: move |_| props.on_change.call(!props.enabled),
            div {
                class: if props.enabled {
                    "w-4 h-4 mt-0.5 rounded border border-white/40 bg-white/20 flex items-center justify-center shrink-0"
                } else {
                    "w-4 h-4 mt-0.5 rounded border border-white/15 bg-transparent flex items-center justify-center shrink-0"
                },
                if props.enabled {
                    i { class: "fa-solid fa-check text-white text-[9px]" }
                }
            }
            div {
                p { class: "text-white text-sm leading-tight", "{props.label}" }
                p { class: "text-slate-600 text-xs mt-0.5", "{props.description}" }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct SearchResultRowProps {
    track: Track,
    on_download: EventHandler<Track>,
}

#[component]
fn SearchResultRow(props: SearchResultRowProps) -> Element {
    let track = &props.track;
    let video_id = track.id.key();
    let active = JOBS.read().iter().any(|job| {
        job.video_id == video_id
            && matches!(
                job.status,
                JobStatus::Pending
                    | JobStatus::Resolving
                    | JobStatus::Downloading
                    | JobStatus::Processing
            )
    });
    let duration = format_duration(track.duration);

    rsx! {
        div { class: "flex items-center gap-3 bg-white/5 hover:bg-white/[0.07] rounded-xl p-2.5 border border-white/10 transition-colors",
            if let Some(cover) = track.cover.as_ref() {
                img {
                    class: "w-12 h-12 rounded-lg object-cover bg-white/5 shrink-0",
                    src: "{cover}",
                    alt: "",
                }
            } else {
                div { class: "w-12 h-12 rounded-lg bg-white/5 flex items-center justify-center shrink-0",
                    i { class: "fa-solid fa-music text-slate-600" }
                }
            }
            div { class: "min-w-0 flex-1",
                p { class: "text-white text-sm truncate", "{track.title}" }
                p { class: "text-slate-500 text-xs truncate",
                    "{track.artist}"
                    if !track.album.is_empty() {
                        span { class: "mx-1.5", "•" }
                        "{track.album}"
                    }
                }
            }
            if !duration.is_empty() {
                span { class: "text-slate-600 text-xs shrink-0", "{duration}" }
            }
            button {
                class: "w-9 h-9 rounded-lg bg-white/10 hover:bg-white/20 disabled:opacity-40 text-white transition-colors shrink-0",
                title: i18n::t("youtube_download_download"),
                disabled: active,
                onclick: move |_| props.on_download.call(props.track.clone()),
                if active {
                    i { class: "fa-solid fa-spinner fa-spin text-xs" }
                } else {
                    i { class: "fa-solid fa-download text-xs" }
                }
            }
        }
    }
}

fn format_duration(seconds: u64) -> String {
    if seconds == 0 {
        String::new()
    } else if seconds >= 3600 {
        format!(
            "{}:{:02}:{:02}",
            seconds / 3600,
            (seconds % 3600) / 60,
            seconds % 60
        )
    } else {
        format!("{}:{:02}", seconds / 60, seconds % 60)
    }
}

#[derive(Props, Clone, PartialEq)]
struct JobRowProps {
    job: DownloadJob,
}

#[component]
fn JobRow(props: JobRowProps) -> Element {
    let job = &props.job;
    let percent = job.progress;
    let (icon, color) = match &job.status {
        JobStatus::Completed => ("fa-solid fa-circle-check", "text-green-400"),
        JobStatus::Resolving | JobStatus::Downloading => {
            ("fa-solid fa-spinner fa-spin", "text-blue-400")
        }
        JobStatus::Processing => ("fa-solid fa-gears", "text-yellow-400"),
        JobStatus::Pending => ("fa-solid fa-clock", "text-slate-500"),
        JobStatus::Failed(_) => ("fa-solid fa-circle-xmark", "text-red-400"),
    };
    let status = match &job.status {
        JobStatus::Resolving => i18n::t("youtube_download_status_resolving"),
        JobStatus::Downloading if !job.speed.is_empty() => i18n::t_with(
            "youtube_download_status_downloading_eta",
            &[
                ("percent", format!("{percent:.0}")),
                ("speed", job.speed.clone()),
                ("eta", job.eta.clone()),
            ],
        ),
        JobStatus::Downloading => i18n::t_with(
            "youtube_download_status_downloading",
            &[("percent", format!("{percent:.0}"))],
        ),
        JobStatus::Processing => i18n::t("youtube_download_status_processing"),
        JobStatus::Completed => i18n::t("youtube_download_status_completed"),
        JobStatus::Pending => i18n::t("youtube_download_status_waiting"),
        JobStatus::Failed(message) => message.clone(),
    };
    let show_progress =
        matches!(job.status, JobStatus::Downloading | JobStatus::Processing) && percent > 0.0;

    rsx! {
        div { class: "bg-white/5 rounded-xl px-4 py-3 border border-white/10",
            div { class: "flex items-start gap-3",
                i { class: "{icon} {color} text-sm mt-0.5 shrink-0" }
                div { class: "flex-1 min-w-0",
                    div { class: "flex items-start justify-between gap-2",
                        span { class: "text-white text-sm truncate flex-1",
                            if job.artist.is_empty() {
                                "{job.title}"
                            } else {
                                "{job.artist} - {job.title}"
                            }
                        }
                        span { class: "text-slate-500 text-xs shrink-0", "{job.format.label()}" }
                    }
                    p {
                        class: if matches!(&job.status, JobStatus::Failed(_)) {
                            "text-red-400 text-xs mt-0.5 truncate"
                        } else {
                            "text-slate-500 text-xs mt-0.5"
                        },
                        "{status}"
                    }
                    if show_progress {
                        div { class: "mt-2 w-full bg-white/10 rounded-full h-1",
                            div {
                                class: if matches!(&job.status, JobStatus::Processing) {
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

#[cfg(test)]
mod tests {
    use super::format_duration;

    #[test]
    fn formats_track_duration() {
        assert_eq!(format_duration(0), "");
        assert_eq!(format_duration(65), "1:05");
        assert_eq!(format_duration(3661), "1:01:01");
    }
}
