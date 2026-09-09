//! Downloading with yt-dlp.
//!
//! A subprocess, a PATH search, an ffmpeg lookup and a filesystem write --
//! system-level work that ran in the UI process, which meant navigating away
//! could kill a download and no other frontend could start or watch one.
//!
//! It runs as an ordinary job here, so progress arrives on the event stream
//! like every other long-running task, and errors are codes rather than
//! translated strings: the client owns the locale.

use std::io::BufRead as _;
use std::path::PathBuf;
use std::sync::Arc;

use api::{ApiError, JobKind, JobRef, YtdlpAudioFormat, YtdlpRequest};

use crate::jobs::{JobCtx, JobRunner};
use crate::session::SessionHandle;

pub struct YtdlpService {
    session: SessionHandle,
    rescan: std::sync::OnceLock<(Arc<crate::library::LibraryService>, Arc<JobRunner>)>,
}

/// Where to look for `yt-dlp` and `ffmpeg`: the inherited PATH, the login
/// shell's PATH, and next to our own binary.
///
/// A desktop app launched from a menu inherits a much shorter PATH than a
/// terminal does, so asking the login shell is what finds a tool the user
/// installed normally.
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

fn find_binary(name: &str) -> Option<String> {
    let exe = if cfg!(target_os = "windows") && !name.ends_with(".exe") {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    search_dirs()
        .iter()
        .map(|dir| dir.join(&exe))
        .find(|candidate| candidate.is_file())
        .map(|path| path.to_string_lossy().into_owned())
}

fn format_args(format: YtdlpAudioFormat) -> Vec<&'static str> {
    match format {
        YtdlpAudioFormat::BestAudio => vec!["-x", "--audio-quality", "0"],
        YtdlpAudioFormat::Mp3 => vec!["-x", "--audio-format", "mp3", "--audio-quality", "0"],
        YtdlpAudioFormat::Flac => vec!["-x", "--audio-format", "flac"],
        YtdlpAudioFormat::Opus => vec!["-x", "--audio-format", "opus"],
        YtdlpAudioFormat::Wav => vec!["-x", "--audio-format", "wav"],
        YtdlpAudioFormat::Video => {
            vec!["-f", "bestvideo+bestaudio", "--merge-output-format", "mp4"]
        }
    }
}

/// The output directory has to exist and be writable before yt-dlp starts,
/// or the failure surfaces a hundred megabytes later.
fn prepare_output(dir: &str) -> Result<(), ApiError> {
    let trimmed = dir.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    let path = PathBuf::from(trimmed);
    if path.exists() && !path.is_dir() {
        return Err(ApiError::invalid_input(
            "the download location is a file, not a folder",
        ));
    }
    std::fs::create_dir_all(&path)
        .map_err(|error| ApiError::invalid_input(format!("cannot use that folder: {error}")))?;
    let probe = path.join(format!(".kopuz-write-test-{}", uuid::Uuid::new_v4()));
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .map_err(|_| ApiError::invalid_input("that folder is not writable"))?;
    let _ = std::fs::remove_file(probe);
    Ok(())
}

fn build_command(request: &YtdlpRequest) -> std::process::Command {
    let binary = find_binary("yt-dlp").unwrap_or_else(|| "yt-dlp".to_string());
    let mut cmd = std::process::Command::new(&binary);
    cmd.env(
        "PATH",
        std::env::join_paths(search_dirs()).unwrap_or_default(),
    );
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }

    let work_dir = if !request.output_dir.is_empty() {
        PathBuf::from(&request.output_dir)
    } else if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home)
    } else {
        PathBuf::from(".")
    };
    if work_dir.is_dir() {
        cmd.current_dir(&work_dir);
    }
    if let Some(ffmpeg) = find_binary("ffmpeg") {
        cmd.arg("--ffmpeg-location").arg(ffmpeg);
    }

    cmd.arg("--newline")
        .arg("--no-warnings")
        .arg("-o")
        .arg("%(album,playlist_title,title)s/%(uploader)s - %(title)s.%(ext)s");
    if !request.output_dir.is_empty() {
        cmd.arg("--paths").arg(&request.output_dir);
    }
    for arg in format_args(request.format) {
        cmd.arg(arg);
    }

    let options = &request.options;
    if request.format != YtdlpAudioFormat::Video {
        cmd.arg("--audio-quality")
            .arg(options.audio_quality.to_string());
    }
    for (enabled, flag) in [
        (options.embed_metadata, "--embed-metadata"),
        (options.embed_thumbnail, "--embed-thumbnail"),
        (options.embed_chapters, "--embed-chapters"),
        (options.embed_subs, "--embed-subs"),
        (options.embed_info_json, "--embed-info-json"),
        (options.write_thumbnail, "--write-thumbnail"),
        (options.write_description, "--write-description"),
        (options.write_info_json, "--write-info-json"),
        (options.write_subs, "--write-subs"),
        (options.write_auto_subs, "--write-auto-subs"),
        (options.write_comments, "--write-comments"),
        (options.split_chapters, "--split-chapters"),
        (options.no_playlist, "--no-playlist"),
        (options.xattrs, "--xattrs"),
        (options.no_mtime, "--no-mtime"),
    ] {
        if enabled {
            cmd.arg(flag);
        }
    }
    if options.sponsorblock {
        cmd.arg("--sponsorblock-remove")
            .arg("sponsor,selfpromo,interaction");
    }
    if options.sponsorblock_mark {
        cmd.arg("--sponsorblock-mark")
            .arg("sponsor,selfpromo,interaction");
    }
    if options.postprocess_thumbnail_square {
        cmd.arg("--convert-thumbnails").arg("png");
        cmd.arg("--postprocessor-args").arg(
            r#"ThumbnailsConvertor+FFmpeg_o:-c:v png -vf crop="'if(gt(ih,iw),iw,ih)':'if(gt(iw,ih),ih,iw)'""#,
        );
    } else if !options.convert_thumbnail.is_empty() {
        cmd.arg("--convert-thumbnails")
            .arg(&options.convert_thumbnail);
    }
    if !options.rate_limit.trim().is_empty() {
        cmd.arg("--limit-rate").arg(options.rate_limit.trim());
    }
    if !options.cookies_from_browser.is_empty() {
        cmd.arg("--cookies-from-browser")
            .arg(&options.cookies_from_browser);
    }
    if !options.js_runtimes.trim().is_empty() {
        cmd.arg("--js-runtimes").arg(options.js_runtimes.trim());
    }

    cmd.arg(&request.url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    cmd
}

/// What one line of yt-dlp's output means.
#[derive(Debug)]
enum Line {
    Progress { percent: f64 },
    Title(String),
    Processing,
    Failed(String),
}

fn parse_line(line: &str) -> Option<Line> {
    let line = line.trim();
    if line.starts_with("ERROR") || line.contains("ERROR:") {
        return Some(Line::Failed(line.to_string()));
    }
    if line.starts_with("[download]") && line.contains('%') && line.contains("at") {
        let percent = line
            .split('%')
            .next()
            .and_then(|part| part.split_whitespace().last())
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or_default();
        return Some(Line::Progress { percent });
    }
    if line.contains("Destination:") {
        let title = line
            .split("Destination:")
            .nth(1)
            .map(|rest| {
                std::path::Path::new(rest.trim())
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_else(|| rest.trim())
                    .to_string()
            })
            .unwrap_or_default();
        if !title.is_empty() {
            return Some(Line::Title(title));
        }
    }
    // Post-processing has no percentage of its own, so the bar holds at 100
    // while ffmpeg works rather than appearing to stall mid-download.
    if line.contains("[ExtractAudio]")
        || line.contains("Deleting original")
        || line.contains("[Merger]")
        || line.contains("[ffmpeg]")
    {
        return Some(Line::Processing);
    }
    None
}

impl YtdlpService {
    pub fn new(session: SessionHandle) -> Arc<Self> {
        Arc::new(Self {
            session,
            rescan: std::sync::OnceLock::new(),
        })
    }

    /// A finished download is a new file under a library root, so the daemon
    /// picks it up itself rather than leaving that to whoever started it.
    pub fn attach_rescan(
        &self,
        library: Arc<crate::library::LibraryService>,
        jobs: Arc<JobRunner>,
    ) {
        let _ = self.rescan.set((library, jobs));
    }

    pub fn start(
        self: &Arc<Self>,
        runner: &JobRunner,
        request: YtdlpRequest,
    ) -> Result<JobRef, ApiError> {
        if request.url.trim().is_empty() {
            return Err(ApiError::invalid_input("a download needs a URL"));
        }
        if find_binary("yt-dlp").is_none() {
            return Err(ApiError::unsupported("yt-dlp is not installed"));
        }
        if find_binary("ffmpeg").is_none() {
            return Err(ApiError::unsupported("ffmpeg is not installed"));
        }
        prepare_output(&request.output_dir)?;
        let service = self.clone();
        runner.start(JobKind::Ytdlp, move |ctx| async move {
            service.run(&ctx, request).await
        })
    }

    async fn run(&self, ctx: &JobCtx, request: YtdlpRequest) -> Result<(), ApiError> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Line>();
        let url = request.url.clone();
        let format = request.format;

        let child = tokio::task::spawn_blocking(move || {
            let mut command = build_command(&request);
            let mut child = command
                .spawn()
                .map_err(|error| format!("yt-dlp would not start: {error}"))?;

            // Drain stderr on its own thread: reading stdout to completion
            // first deadlocks if yt-dlp fills the stderr pipe.
            let errors = child.stderr.take().map(|stderr| {
                std::thread::spawn(move || {
                    std::io::BufReader::new(stderr)
                        .lines()
                        .map_while(Result::ok)
                        .filter(|line| line.contains("ERROR"))
                        .collect::<Vec<String>>()
                })
            });
            if let Some(stdout) = child.stdout.take() {
                for line in std::io::BufReader::new(stdout)
                    .lines()
                    .map_while(Result::ok)
                {
                    if let Some(parsed) = parse_line(&line) {
                        let _ = tx.send(parsed);
                    }
                }
            }
            let errors = errors
                .map(|thread| thread.join().unwrap_or_default())
                .unwrap_or_default();
            match child.wait() {
                Ok(status) if status.success() => Ok(()),
                Ok(status) if !errors.is_empty() => {
                    let _ = status;
                    Err(errors.join("\n"))
                }
                Ok(status) => Err(format!("yt-dlp exited with {status}")),
                Err(error) => Err(format!("yt-dlp could not be waited on: {error}")),
            }
        });

        let mut title = url.clone();
        let mut failure = None;
        while let Some(line) = rx.recv().await {
            match line {
                Line::Title(found) => {
                    title = found;
                    ctx.progress("downloading", Some(0), Some(100), Some(title.clone()));
                }
                Line::Progress { percent, .. } => {
                    ctx.progress_throttled(
                        "downloading",
                        Some(percent.round().clamp(0.0, 100.0) as u64),
                        Some(100),
                        Some(title.clone()),
                    );
                }
                Line::Processing => {
                    ctx.progress("processing", Some(100), Some(100), Some(title.clone()));
                }
                Line::Failed(message) => failure = Some(message),
            }
        }

        let outcome = child
            .await
            .map_err(|error| ApiError::internal(format!("the download task failed: {error}")))?;
        let result = match (outcome, failure) {
            (Ok(()), None) => Ok(()),
            (Ok(()), Some(message)) | (Err(message), _) => Err(message),
        };
        self.record_history(&url, &title, format, result.as_ref().err().cloned())
            .await;
        if result.is_ok()
            && let Some((library, jobs)) = self.rescan.get()
            && let Err(error) = library.spawn_scan(jobs)
        {
            tracing::debug!(%error, "no rescan after the download");
        }
        result.map_err(ApiError::internal)
    }

    /// Downloads are remembered so the page can show what happened after a
    /// restart, which is why this is config rather than a job list.
    async fn record_history(
        &self,
        url: &str,
        title: &str,
        format: YtdlpAudioFormat,
        error: Option<String>,
    ) {
        let entry = config::YtdlpHistoryEntry {
            url: url.to_string(),
            title: title.to_string(),
            format: format_label(format).to_string(),
            status: if error.is_some() {
                "failed".to_string()
            } else {
                "completed".to_string()
            },
            error,
        };
        let mut config = self.session.config_watch().borrow().clone();
        config.ytdlp_history.insert(0, entry);
        config.ytdlp_history.truncate(50);
        self.session
            .set_config(config, vec!["ytdlp_history".to_string()]);
    }
}

/// The label the history stores, which is what the page renders back.
fn format_label(format: YtdlpAudioFormat) -> &'static str {
    match format {
        YtdlpAudioFormat::BestAudio => "Best Audio",
        YtdlpAudioFormat::Mp3 => "MP3",
        YtdlpAudioFormat::Flac => "FLAC",
        YtdlpAudioFormat::Opus => "OPUS",
        YtdlpAudioFormat::Wav => "WAV",
        YtdlpAudioFormat::Video => "Video (MP4)",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The progress line is the only structured output yt-dlp gives, and it
    /// is whitespace-formatted text -- worth pinning.
    #[test]
    fn a_download_progress_line_yields_a_percentage() {
        let line = "[download]  42.5% of 5.00MiB at 1.20MiB/s ETA 00:03";
        match parse_line(line).expect("a progress line") {
            Line::Progress { percent } => assert!((percent - 42.5).abs() < f64::EPSILON),
            other => panic!("expected progress, got {other:?}"),
        }
    }

    #[test]
    fn destination_names_the_track_and_errors_are_failures() {
        match parse_line("[download] Destination: /music/Album/Artist - Song.opus") {
            Some(Line::Title(title)) => assert_eq!(title, "Artist - Song.opus"),
            other => panic!("expected a title, got {other:?}"),
        }
        assert!(matches!(
            parse_line("ERROR: Video unavailable"),
            Some(Line::Failed(_))
        ));
        assert!(parse_line("[youtube] Extracting URL").is_none());
    }
}
