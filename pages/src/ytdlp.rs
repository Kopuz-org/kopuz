use config::{AppConfig, YtdlpOptions};
use dioxus::prelude::*;
use std::fs::{self, OpenOptions};
use std::io::BufRead;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct DownloadJob {
    pub id: String,
    pub url: String,
    pub title: String,
    pub format: AudioFormat,
    pub progress: f64,
    pub status: JobStatus,
    pub speed: String,
    pub eta: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum JobStatus {
    Pending,
    Downloading,
    Processing,
    Completed,
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Copy)]
pub enum AudioFormat {
    BestAudio,
    Mp3,
    Flac,
    Wav,
    Video,
}

impl AudioFormat {
    fn label_key(self) -> &'static str {
        match self {
            Self::BestAudio => "ytdlp_format_best_audio",
            Self::Mp3 => "ytdlp_format_mp3",
            Self::Flac => "ytdlp_format_flac",
            Self::Wav => "ytdlp_format_wav",
            Self::Video => "ytdlp_format_video",
        }
    }

    fn label(self) -> String {
        i18n::t(self.label_key())
    }

    fn storage_label(self) -> &'static str {
        match self {
            Self::BestAudio => "Best Audio",
            Self::Mp3 => "MP3",
            Self::Flac => "FLAC",
            Self::Wav => "WAV",
            Self::Video => "Video (MP4)",
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "MP3" => Self::Mp3,
            "FLAC" => Self::Flac,
            "WAV" => Self::Wav,
            "Video (MP4)" => Self::Video,
            _ => Self::BestAudio,
        }
    }

    fn ytdlp_args(self) -> Vec<&'static str> {
        match self {
            Self::BestAudio => vec!["-x", "--audio-quality", "0"],
            Self::Mp3 => vec!["-x", "--audio-format", "mp3", "--audio-quality", "0"],
            Self::Flac => vec!["-x", "--audio-format", "flac"],
            Self::Wav => vec!["-x", "--audio-format", "wav"],
            Self::Video => vec!["-f", "bestvideo+bestaudio", "--merge-output-format", "mp4"],
        }
    }
}

fn find_ytdlp() -> String {
    let static_candidates: &[&str] = &[
        "/opt/homebrew/bin/yt-dlp",
        "/usr/local/bin/yt-dlp",
        "/usr/bin/yt-dlp",
        "/snap/bin/yt-dlp",
        "/usr/local/sbin/yt-dlp",
    ];

    for path in static_candidates {
        if std::path::Path::new(path).exists() {
            return path.to_string();
        }
    }

    if let Some(home) = std::env::var_os("HOME") {
        let p = std::path::PathBuf::from(&home).join(".local/bin/yt-dlp");
        if p.exists() {
            return p.to_string_lossy().into_owned();
        }
        for ver in &["3.13", "3.12", "3.11", "3.10", "3.9"] {
            let p =
                std::path::PathBuf::from(&home).join(format!("Library/Python/{}/bin/yt-dlp", ver));
            if p.exists() {
                return p.to_string_lossy().into_owned();
            }
        }
    }

    if let Some(found) = find_in_augmented_path("yt-dlp") {
        return found;
    }

    "yt-dlp".to_string()
}

fn augmented_path() -> String {
    format!(
        "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:{}",
        std::env::var("PATH").unwrap_or_default()
    )
}

fn find_in_augmented_path(binary: &str) -> Option<String> {
    let binary_path = Path::new(binary);
    if binary_path.is_absolute() && binary_path.exists() {
        return Some(binary.to_string());
    }

    let mut candidates = vec![binary.to_string()];
    #[cfg(target_os = "windows")]
    if !binary.ends_with(".exe") {
        candidates.push(format!("{binary}.exe"));
    }

    for dir in std::env::split_paths(&augmented_path()) {
        for candidate in &candidates {
            let path = dir.join(candidate);
            if path.exists() {
                return Some(path.to_string_lossy().into_owned());
            }
        }
    }

    None
}

fn validate_output_directory(out_dir: &str) -> Result<(), String> {
    let trimmed = out_dir.trim();
    if trimmed.is_empty() {
        return Ok(());
    }

    let path = PathBuf::from(trimmed);
    if path.exists() && !path.is_dir() {
        return Err(i18n::t_with(
            "ytdlp_error_output_not_directory",
            &[("path", trimmed.to_string())],
        ));
    }

    fs::create_dir_all(&path).map_err(|error| {
        i18n::t_with(
            "ytdlp_error_output_prepare",
            &[("error", error.to_string())],
        )
    })?;

    let probe_path = path.join(format!(".kopuz-write-test-{}", uuid::Uuid::new_v4()));
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe_path)
        .map_err(|_| {
            i18n::t_with(
                "ytdlp_error_output_not_writable",
                &[("path", trimmed.to_string())],
            )
        })?;
    let _ = fs::remove_file(probe_path);

    Ok(())
}

fn run_preflight_checks(url: &str, out_dir: &str, jobs: &[DownloadJob]) -> Result<(), String> {
    if jobs.iter().any(|job| {
        job.url.trim() == url
            && matches!(
                job.status,
                JobStatus::Pending | JobStatus::Downloading | JobStatus::Processing
            )
    }) {
        return Err(i18n::t("ytdlp_error_duplicate_active"));
    }

    if find_in_augmented_path(&find_ytdlp()).is_none() {
        return Err(i18n::t("ytdlp_error_not_found"));
    }

    if find_in_augmented_path("ffmpeg").is_none() {
        return Err(i18n::t("ytdlp_error_ffmpeg_not_found"));
    }

    validate_output_directory(out_dir)
}

fn build_command(
    url: &str,
    out: &str,
    fmt: AudioFormat,
    opts: &YtdlpOptions,
) -> std::process::Command {
    let binary = find_ytdlp();
    let mut cmd = std::process::Command::new(&binary);
    cmd.env("PATH", augmented_path());

    cmd.arg("--newline")
        .arg("--no-warnings")
        .arg("-o")
        .arg("%(album,playlist_title,title)s/%(uploader)s - %(title)s.%(ext)s");

    if !out.is_empty() {
        cmd.arg("--paths").arg(out);
    }

    for arg in fmt.ytdlp_args() {
        cmd.arg(arg);
    }

    if matches!(
        fmt,
        AudioFormat::Mp3 | AudioFormat::Flac | AudioFormat::Wav | AudioFormat::BestAudio
    ) {
        cmd.arg("--audio-quality")
            .arg(opts.audio_quality.to_string());
    }

    if opts.embed_metadata {
        cmd.arg("--embed-metadata");
    }
    if opts.embed_thumbnail {
        cmd.arg("--embed-thumbnail");
    }
    if opts.embed_chapters {
        cmd.arg("--embed-chapters");
    }
    if opts.embed_subs {
        cmd.arg("--embed-subs");
    }
    if opts.embed_info_json {
        cmd.arg("--embed-info-json");
    }

    if opts.write_thumbnail {
        cmd.arg("--write-thumbnail");
    }
    if opts.write_description {
        cmd.arg("--write-description");
    }
    if opts.write_info_json {
        cmd.arg("--write-info-json");
    }
    if opts.write_subs {
        cmd.arg("--write-subs");
    }
    if opts.write_auto_subs {
        cmd.arg("--write-auto-subs");
    }
    if opts.write_comments {
        cmd.arg("--write-comments");
    }

    if opts.sponsorblock {
        cmd.arg("--sponsorblock-remove")
            .arg("sponsor,selfpromo,interaction");
    }
    if opts.sponsorblock_mark {
        cmd.arg("--sponsorblock-mark")
            .arg("sponsor,selfpromo,interaction");
    }
    if opts.split_chapters {
        cmd.arg("--split-chapters");
    }
    if opts.postprocess_thumbnail_square {
        cmd.arg("--convert-thumbnails").arg("png");
        cmd.arg("--postprocessor-args").arg(
            r#"ThumbnailsConvertor+FFmpeg_o:-c:v png -vf crop="'if(gt(ih,iw),iw,ih)':'if(gt(iw,ih),ih,iw)'""#,
        );
    } else if !opts.convert_thumbnail.is_empty() {
        cmd.arg("--convert-thumbnails").arg(&opts.convert_thumbnail);
    }

    if opts.no_playlist {
        cmd.arg("--no-playlist");
    }
    if opts.xattrs {
        cmd.arg("--xattrs");
    }
    if opts.no_mtime {
        cmd.arg("--no-mtime");
    }
    if !opts.rate_limit.trim().is_empty() {
        cmd.arg("--limit-rate").arg(opts.rate_limit.trim());
    }
    if !opts.cookies_from_browser.is_empty() {
        cmd.arg("--cookies-from-browser")
            .arg(&opts.cookies_from_browser);
    }

    cmd.arg(url)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    cmd
}

enum LineInfo {
    Progress {
        pct: f64,
        speed: String,
        eta: String,
    },
    Title(String),
    Processing,
    Done,
    Error(String),
}

fn parse_line(line: &str) -> Option<LineInfo> {
    let line = line.trim();

    if line.starts_with("ERROR") || line.contains("ERROR:") {
        return Some(LineInfo::Error(line.to_string()));
    }
    if line.starts_with("[download]") && line.contains('%') && line.contains("at") {
        let pct = line
            .split('%')
            .next()
            .and_then(|s| s.split_whitespace().last())
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);
        let speed = line
            .split("at")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .unwrap_or("")
            .to_string();
        let eta = line
            .split("ETA")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .unwrap_or("")
            .to_string();
        return Some(LineInfo::Progress { pct, speed, eta });
    }
    if line.starts_with("[download]") && line.contains("100%") {
        return Some(LineInfo::Progress {
            pct: 100.0,
            speed: String::new(),
            eta: String::new(),
        });
    }
    if line.contains("Destination:") {
        let title = line
            .split("Destination:")
            .nth(1)
            .map(|s| {
                std::path::Path::new(s.trim())
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(s.trim())
                    .to_string()
            })
            .unwrap_or_default();
        if !title.is_empty() {
            return Some(LineInfo::Title(title));
        }
    }
    if line.contains("[ExtractAudio]")
        || line.contains("Deleting original")
        || line.contains("[Merger]")
        || line.contains("[ffmpeg]")
    {
        return Some(LineInfo::Processing);
    }
    None
}

#[component]
pub fn YtdlpPage(config: Signal<AppConfig>, mut trigger_rescan: Signal<usize>) -> Element {
    let mut url_input = use_signal(String::new);
    let mut format = use_signal(|| AudioFormat::BestAudio);
    let mut jobs = use_signal(|| Vec::<DownloadJob>::new());
    let mut out_dir = use_signal(|| config.peek().ytdlp_output_dir.clone());
    let mut show_opts = use_signal(|| false);
    let mut preflight_error = use_signal(|| Option::<String>::None);

    use_hook(move || {
        let history = config.peek().ytdlp_history.clone();
        jobs.set(
            history
                .iter()
                .map(|e| DownloadJob {
                    id: uuid::Uuid::new_v4().to_string(),
                    url: e.url.clone(),
                    title: e.title.clone(),
                    format: AudioFormat::from_str(&e.format),
                    progress: if e.status == "completed" { 100.0 } else { 0.0 },
                    status: if e.status == "completed" {
                        JobStatus::Completed
                    } else {
                        JobStatus::Failed(e.error.clone().unwrap_or_default())
                    },
                    speed: String::new(),
                    eta: String::new(),
                })
                .collect(),
        );
    });

    let mut do_download = move || {
        let url = url_input().trim().to_string();
        if url.is_empty() {
            return;
        }

        preflight_error.set(None);

        if let Err(error) = run_preflight_checks(&url, &out_dir(), &jobs.read()) {
            preflight_error.set(Some(error));
            return;
        }

        let out = out_dir();
        let fmt = format();
        let opts = config.peek().ytdlp_options.clone();
        let job_id = uuid::Uuid::new_v4().to_string();

        jobs.write().insert(
            0,
            DownloadJob {
                id: job_id.clone(),
                url: url.clone(),
                title: url.clone(),
                format: fmt,
                progress: 0.0,
                status: JobStatus::Pending,
                speed: String::new(),
                eta: String::new(),
            },
        );
        url_input.set(String::new());

        spawn(async move {
            if let Some(j) = jobs.write().iter_mut().find(|j| j.id == job_id) {
                j.status = JobStatus::Downloading;
            }

            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<LineInfo>();

            tokio::task::spawn_blocking(move || {
                let mut cmd = build_command(&url, &out, fmt, &opts);

                let mut child = match cmd.spawn() {
                    Ok(c) => c,
                    Err(e) => {
                        let msg = if e.kind() == std::io::ErrorKind::NotFound {
                            i18n::t("ytdlp_error_not_found")
                        } else {
                            i18n::t_with("ytdlp_error_start", &[("error", e.to_string())])
                        };
                        let _ = tx.send(LineInfo::Error(msg));
                        return;
                    }
                };

                if let Some(stdout) = child.stdout.take() {
                    for line in std::io::BufReader::new(stdout).lines().flatten() {
                        if let Some(info) = parse_line(&line) {
                            let _ = tx.send(info);
                        }
                    }
                }
                if let Some(stderr) = child.stderr.take() {
                    let errs: Vec<String> = std::io::BufReader::new(stderr)
                        .lines()
                        .flatten()
                        .filter(|l| l.contains("ERROR"))
                        .collect();
                    if !errs.is_empty() {
                        let _ = tx.send(LineInfo::Error(errs.join("\n")));
                    }
                }
                match child.wait() {
                    Ok(s) if s.success() => {
                        let _ = tx.send(LineInfo::Done);
                    }
                    Ok(s) => {
                        let _ = tx.send(LineInfo::Error(i18n::t_with(
                            "ytdlp_error_exit",
                            &[("status", s.to_string())],
                        )));
                    }
                    Err(e) => {
                        let _ = tx.send(LineInfo::Error(e.to_string()));
                    }
                }
            });

            while let Some(info) = rx.recv().await {
                let id = &job_id;
                match info {
                    LineInfo::Progress { pct, speed, eta } => {
                        if let Some(j) = jobs.write().iter_mut().find(|j| &j.id == id) {
                            j.progress = pct;
                            j.speed = speed;
                            j.eta = eta;
                            j.status = JobStatus::Downloading;
                        }
                    }
                    LineInfo::Title(title) => {
                        if let Some(j) = jobs.write().iter_mut().find(|j| &j.id == id) {
                            j.title = title;
                        }
                    }
                    LineInfo::Processing => {
                        if let Some(j) = jobs.write().iter_mut().find(|j| &j.id == id) {
                            j.status = JobStatus::Processing;
                            j.progress = 100.0;
                        }
                    }
                    LineInfo::Done => {
                        let entry = jobs.read().iter().find(|j| &j.id == id).map(|j| {
                            config::YtdlpHistoryEntry {
                                url: j.url.clone(),
                                title: j.title.clone(),
                                format: j.format.storage_label().to_string(),
                                status: "completed".into(),
                                error: None,
                            }
                        });
                        if let Some(j) = jobs.write().iter_mut().find(|j| &j.id == id) {
                            j.status = JobStatus::Completed;
                            j.progress = 100.0;
                            j.speed = String::new();
                            j.eta = String::new();
                        }
                        if let Some(e) = entry {
                            let mut cfg = config.write();
                            cfg.ytdlp_history.insert(0, e);
                            cfg.ytdlp_history.truncate(200);
                        }
                        *trigger_rescan.write() += 1;
                        break;
                    }
                    LineInfo::Error(msg) => {
                        let entry = jobs.read().iter().find(|j| &j.id == id).map(|j| {
                            config::YtdlpHistoryEntry {
                                url: j.url.clone(),
                                title: j.title.clone(),
                                format: j.format.storage_label().to_string(),
                                status: "failed".into(),
                                error: Some(msg.clone()),
                            }
                        });
                        if let Some(j) = jobs.write().iter_mut().find(|j| &j.id == id) {
                            j.status = JobStatus::Failed(msg);
                        }
                        if let Some(e) = entry {
                            let mut cfg = config.write();
                            cfg.ytdlp_history.insert(0, e);
                            cfg.ytdlp_history.truncate(200);
                        }
                        break;
                    }
                }
            }
        });
    };

    rsx! {
        div { class: "p-6 max-w-3xl mx-auto",

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
                        preflight_error.set(None);
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
                for fmt in [AudioFormat::BestAudio, AudioFormat::Mp3, AudioFormat::Flac, AudioFormat::Wav, AudioFormat::Video] {
                    button {
                        class: if *format.read() == fmt {
                            "text-xs px-3 py-1.5 rounded-lg bg-white/20 text-white font-medium transition-colors"
                        } else {
                            "text-xs px-3 py-1.5 rounded-lg bg-white/5 text-slate-400 hover:text-white hover:bg-white/10 transition-colors"
                        },
                        onclick: move |_| format.set(fmt),
                        "{fmt.label()}"
                    }
                }
            }

            if let Some(error) = preflight_error.read().clone() {
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
                        preflight_error.set(None);
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

            if !jobs.read().is_empty() {
                div { class: "space-y-2 mt-2",
                    div { class: "flex justify-end mb-1",
                        button {
                            class: "text-slate-600 hover:text-slate-400 text-xs transition-colors",
                            onclick: move |_| {
                                jobs.write().retain(|j| matches!(
                                    j.status,
                                    JobStatus::Downloading | JobStatus::Processing | JobStatus::Pending
                                ));
                                config.write().ytdlp_history.clear();
                            },
                            "{i18n::t(\"ytdlp_clear_history\")}"
                        }
                    }
                    for job in jobs.read().clone().into_iter() {
                        JobRow { job }
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

#[component]
fn OptionsPanel(config: Signal<AppConfig>) -> Element {
    let opts = use_memo(move || config.read().ytdlp_options.clone());

    rsx! {
        div { class: "bg-white/5 border border-white/10 rounded-xl p-5 mb-5 space-y-5",

            div {
                p { class: "text-xs font-semibold text-slate-400 uppercase tracking-wider mb-3",
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
                p { class: "text-xs font-semibold text-slate-400 uppercase tracking-wider mb-3",
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
                p { class: "text-xs font-semibold text-slate-400 uppercase tracking-wider mb-3",
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
                p { class: "text-xs font-semibold text-slate-400 uppercase tracking-wider mb-3",
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

#[derive(Props, Clone, PartialEq)]
struct JobRowProps {
    job: DownloadJob,
}

#[component]
fn JobRow(props: JobRowProps) -> Element {
    let job = &props.job;
    let pct = job.progress;

    let (icon, icon_color) = match &job.status {
        JobStatus::Completed => ("fa-solid fa-circle-check", "text-green-400"),
        JobStatus::Downloading => ("fa-solid fa-spinner fa-spin", "text-blue-400"),
        JobStatus::Processing => ("fa-solid fa-gears", "text-yellow-400"),
        JobStatus::Pending => ("fa-solid fa-clock", "text-slate-500"),
        JobStatus::Failed(_) => ("fa-solid fa-circle-xmark", "text-red-400"),
    };

    let status_text = match &job.status {
        JobStatus::Downloading if !job.speed.is_empty() => i18n::t_with(
            "ytdlp_status_downloading_eta",
            &[
                ("percent", format!("{pct:.0}")),
                ("speed", job.speed.clone()),
                ("eta", job.eta.clone()),
            ],
        ),
        JobStatus::Downloading => i18n::t_with(
            "ytdlp_status_downloading",
            &[("percent", format!("{pct:.0}"))],
        ),
        JobStatus::Processing => i18n::t("ytdlp_status_processing"),
        JobStatus::Completed => i18n::t("ytdlp_status_completed"),
        JobStatus::Pending => i18n::t("ytdlp_status_waiting"),
        JobStatus::Failed(msg) => msg.clone(),
    };

    let title = if job.title == job.url {
        job.url
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .chars()
            .take(60)
            .collect::<String>()
    } else {
        job.title.clone()
    };

    let show_bar =
        matches!(job.status, JobStatus::Downloading | JobStatus::Processing) && pct > 0.0;

    rsx! {
        div { class: "bg-white/5 rounded-xl px-4 py-3 border border-white/10",
            div { class: "flex items-start gap-3",
                i { class: "{icon} {icon_color} text-sm mt-0.5 shrink-0" }
                div { class: "flex-1 min-w-0",
                    div { class: "flex items-start justify-between gap-2",
                        span { class: "text-white text-sm truncate flex-1", "{title}" }
                        span { class: "text-slate-500 text-xs shrink-0", "{job.format.label()}" }
                    }
                    p {
                        class: if matches!(&job.status, JobStatus::Failed(_)) {
                            "text-red-400 text-xs mt-0.5 truncate"
                        } else {
                            "text-slate-500 text-xs mt-0.5"
                        },
                        "{status_text}"
                    }
                    if show_bar {
                        div { class: "mt-2 w-full bg-white/10 rounded-full h-1",
                            div {
                                class: if matches!(&job.status, JobStatus::Processing) {
                                    "h-1 rounded-full bg-yellow-400/60 transition-all duration-300"
                                } else {
                                    "h-1 rounded-full bg-white/50 transition-all duration-300"
                                },
                                style: "width: {pct:.1}%"
                            }
                        }
                    }
                }
            }
        }
    }
}
