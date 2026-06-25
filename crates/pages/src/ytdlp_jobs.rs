use config::{AppConfig, YtdlpOptions};
use dioxus::core::spawn_forever;
use dioxus::prelude::*;
use std::fs::{self, OpenOptions};
use std::io::BufRead;
use std::path::PathBuf;

/// App-lifetime job list: downloads keep running (and keep their live
/// progress) when the user navigates away from the page (#327).
pub(crate) static JOBS: GlobalSignal<Vec<DownloadJob>> = Signal::global(Vec::new);

/// Completions waiting to be applied to config history / the rescan trigger.
static FINISHED: GlobalSignal<Vec<(config::YtdlpHistoryEntry, bool)>> = Signal::global(Vec::new);

/// Install in App: applies finished yt-dlp jobs to the config history and
/// bumps the rescan trigger for successful ones.
pub fn use_ytdlp_completion_sink(mut config: Signal<AppConfig>, mut trigger_rescan: Signal<usize>) {
    use_effect(move || {
        if FINISHED.read().is_empty() {
            return;
        }
        let drained: Vec<_> = FINISHED.write().drain(..).collect();
        let mut rescan = false;
        {
            let mut cfg = config.write();
            for (entry, ok) in drained {
                rescan |= ok;
                cfg.ytdlp_history.insert(0, entry);
                cfg.ytdlp_history.truncate(200);
            }
        }
        if rescan {
            *trigger_rescan.write() += 1;
        }
    });
}

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

    pub fn label(self) -> String {
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

pub fn seed_from_history(history: &[config::YtdlpHistoryEntry]) {
    if !JOBS.read().is_empty() {
        return;
    }

    *JOBS.write() = history
        .iter()
        .map(|entry| DownloadJob {
            id: uuid::Uuid::new_v4().to_string(),
            url: entry.url.clone(),
            title: entry.title.clone(),
            format: AudioFormat::from_str(&entry.format),
            progress: if entry.status == "completed" {
                100.0
            } else {
                0.0
            },
            status: if entry.status == "completed" {
                JobStatus::Completed
            } else {
                JobStatus::Failed(entry.error.clone().unwrap_or_default())
            },
            speed: String::new(),
            eta: String::new(),
        })
        .collect();
}

pub fn clear_finished_jobs() {
    JOBS.write().retain(|job| {
        matches!(
            job.status,
            JobStatus::Downloading | JobStatus::Processing | JobStatus::Pending
        )
    });
}

pub fn run_preflight_checks(url: &str, out_dir: &str) -> Result<(), String> {
    if JOBS.read().iter().any(|job| {
        job.url.trim() == url
            && matches!(
                job.status,
                JobStatus::Pending | JobStatus::Downloading | JobStatus::Processing
            )
    }) {
        return Err(i18n::t("ytdlp_error_duplicate_active"));
    }

    if find_binary("yt-dlp").is_none() {
        return Err(i18n::t("ytdlp_error_not_found"));
    }

    if find_ffmpeg().is_none() {
        return Err(i18n::t("ytdlp_error_ffmpeg_not_found"));
    }

    validate_output_directory(out_dir)
}

pub fn start_download(url: String, out: String, fmt: AudioFormat, opts: YtdlpOptions) {
    let job_id = uuid::Uuid::new_v4().to_string();

    JOBS.write().insert(
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

    // spawn_forever: the driver must outlive the page or navigating away kills
    // the download mid-flight (#327). It only writes globals (JOBS, FINISHED).
    spawn_forever(async move {
        if let Some(job) = JOBS.write().iter_mut().find(|job| job.id == job_id) {
            job.status = JobStatus::Downloading;
        }

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<LineInfo>();

        tokio::task::spawn_blocking(move || {
            let mut cmd = build_command(&url, &out, fmt, &opts);

            let mut child = match cmd.spawn() {
                Ok(child) => child,
                Err(error) => {
                    let not_found = error.kind() == std::io::ErrorKind::NotFound;
                    tracing::error!(target: "ytdlp", error = %error, not_found, "yt-dlp: spawn failed");
                    let msg = if not_found {
                        i18n::t("ytdlp_error_not_found")
                    } else {
                        i18n::t_with("ytdlp_error_start", &[("error", error.to_string())])
                    };
                    let _ = tx.send(LineInfo::Error(msg));
                    return;
                }
            };

            // Drain stderr on its own thread: reading stdout to completion first
            // and stderr second deadlocks if yt-dlp fills the stderr pipe.
            let stderr_thread = child.stderr.take().map(|stderr| {
                std::thread::spawn(move || {
                    std::io::BufReader::new(stderr)
                        .lines()
                        .map_while(Result::ok)
                        .filter(|line| line.contains("ERROR"))
                        .collect::<Vec<_>>()
                })
            });

            if let Some(stdout) = child.stdout.take() {
                for line in std::io::BufReader::new(stdout)
                    .lines()
                    .map_while(Result::ok)
                {
                    if let Some(info) = parse_line(&line) {
                        let _ = tx.send(info);
                    }
                }
            }

            let errs = stderr_thread
                .map(|thread| thread.join().unwrap_or_default())
                .unwrap_or_default();
            if !errs.is_empty() {
                tracing::warn!(target: "ytdlp", "yt-dlp stderr: {}", errs.join(" | "));
                let _ = tx.send(LineInfo::Error(errs.join("\n")));
            }
            match child.wait() {
                Ok(status) if status.success() => {
                    tracing::info!(target: "ytdlp", "yt-dlp: download finished");
                    let _ = tx.send(LineInfo::Done);
                }
                Ok(status) => {
                    tracing::warn!(target: "ytdlp", status = %status, "yt-dlp: exited non-zero");
                    let _ = tx.send(LineInfo::Error(i18n::t_with(
                        "ytdlp_error_exit",
                        &[("status", status.to_string())],
                    )));
                }
                Err(error) => {
                    tracing::error!(target: "ytdlp", error = %error, "yt-dlp: wait failed");
                    let _ = tx.send(LineInfo::Error(error.to_string()));
                }
            }
        });

        while let Some(info) = rx.recv().await {
            let id = &job_id;
            match info {
                LineInfo::Progress { pct, speed, eta } => {
                    if let Some(job) = JOBS.write().iter_mut().find(|job| &job.id == id) {
                        job.progress = pct;
                        job.speed = speed;
                        job.eta = eta;
                        job.status = JobStatus::Downloading;
                    }
                }
                LineInfo::Title(title) => {
                    if let Some(job) = JOBS.write().iter_mut().find(|job| &job.id == id) {
                        job.title = title;
                    }
                }
                LineInfo::Processing => {
                    if let Some(job) = JOBS.write().iter_mut().find(|job| &job.id == id) {
                        job.status = JobStatus::Processing;
                        job.progress = 100.0;
                    }
                }
                LineInfo::Done => {
                    let entry = JOBS.read().iter().find(|job| &job.id == id).map(|job| {
                        config::YtdlpHistoryEntry {
                            url: job.url.clone(),
                            title: job.title.clone(),
                            format: job.format.storage_label().to_string(),
                            status: "completed".into(),
                            error: None,
                        }
                    });
                    if let Some(job) = JOBS.write().iter_mut().find(|job| &job.id == id) {
                        job.status = JobStatus::Completed;
                        job.progress = 100.0;
                        job.speed = String::new();
                        job.eta = String::new();
                    }
                    if let Some(entry) = entry {
                        FINISHED.write().push((entry, true));
                    }
                    break;
                }
                LineInfo::Error(msg) => {
                    let entry = JOBS.read().iter().find(|job| &job.id == id).map(|job| {
                        config::YtdlpHistoryEntry {
                            url: job.url.clone(),
                            title: job.title.clone(),
                            format: job.format.storage_label().to_string(),
                            status: "failed".into(),
                            error: Some(msg.clone()),
                        }
                    });
                    if let Some(job) = JOBS.write().iter_mut().find(|job| &job.id == id) {
                        job.status = JobStatus::Failed(msg);
                    }
                    if let Some(entry) = entry {
                        FINISHED.write().push((entry, false));
                    }
                    break;
                }
            }
        }
    });
}

fn search_dirs() -> &'static [PathBuf] {
    static DIRS: std::sync::OnceLock<Vec<PathBuf>> = std::sync::OnceLock::new();
    DIRS.get_or_init(|| {
        let mut dirs: Vec<PathBuf> =
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect();

        if let Some(shell) = std::env::var_os("SHELL")
            && let Ok(out) = std::process::Command::new(shell)
                .arg("-lc")
                .arg("printf %s \"$PATH\"")
                .output()
            && out.status.success()
        {
            let path = String::from_utf8_lossy(&out.stdout);
            for dir in std::env::split_paths(path.trim()) {
                if !dirs.contains(&dir) {
                    dirs.push(dir);
                }
            }
        }

        if let Ok(exe) = std::env::current_exe()
            && let Some(exe_dir) = exe.parent()
            && !dirs.iter().any(|dir| dir == exe_dir)
        {
            dirs.push(exe_dir.to_path_buf());
        }

        dirs
    })
}

fn augmented_path() -> std::ffi::OsString {
    std::env::join_paths(search_dirs()).unwrap_or_default()
}

fn find_binary(name: &str) -> Option<String> {
    let exe = if cfg!(target_os = "windows") && !name.ends_with(".exe") {
        format!("{name}.exe")
    } else {
        name.to_string()
    };

    for dir in search_dirs() {
        let candidate = dir.join(&exe);
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }
    None
}

fn find_ytdlp() -> String {
    find_binary("yt-dlp").unwrap_or_else(|| "yt-dlp".to_string())
}

fn find_ffmpeg() -> Option<String> {
    find_binary("ffmpeg")
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

fn build_command(
    url: &str,
    out: &str,
    fmt: AudioFormat,
    opts: &YtdlpOptions,
) -> std::process::Command {
    let binary = find_ytdlp();
    let mut cmd = std::process::Command::new(&binary);
    cmd.env("PATH", augmented_path());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }

    let work_dir = if !out.is_empty() {
        PathBuf::from(out)
    } else if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home)
    } else {
        PathBuf::from(".")
    };
    if work_dir.is_dir() {
        cmd.current_dir(&work_dir);
    }

    if let Some(ffmpeg) = find_ffmpeg() {
        cmd.arg("--ffmpeg-location").arg(ffmpeg);
    }

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
        cmd.arg("--postprocessor-args")
            .arg(r#"ThumbnailsConvertor+FFmpeg_o:-c:v png -vf crop="'if(gt(ih,iw),iw,ih)':'if(gt(iw,ih),ih,iw)'""#);
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
    if !opts.js_runtimes.trim().is_empty() {
        cmd.arg("--js-runtimes").arg(opts.js_runtimes.trim());
    }

    cmd.arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let args: Vec<String> = cmd
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    tracing::info!(
        target: "ytdlp",
        binary = %binary,
        located = std::path::Path::new(&binary).is_absolute(),
        cwd = ?cmd.get_current_dir(),
        args = ?args,
        "yt-dlp: built download command"
    );

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
                    .and_then(|name| name.to_str())
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
