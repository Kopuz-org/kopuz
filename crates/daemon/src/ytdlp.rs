use std::fs::{self, OpenOptions};
use std::io::BufRead as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use api::{ApiError, JobKind, JobRef};

use crate::{ConfigService, JobRunner, LibraryService, SessionHandle};

enum OutputLine {
    Line(String),
    Stderr(String),
}

fn search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> =
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect();
    if let Some(shell) = std::env::var_os("SHELL")
        && let Ok(output) = Command::new(shell)
            .arg("-lc")
            .arg("printf %s \"$PATH\"")
            .output()
        && output.status.success()
    {
        for dir in std::env::split_paths(String::from_utf8_lossy(&output.stdout).trim()) {
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
    }
    if let Ok(executable) = std::env::current_exe()
        && let Some(dir) = executable.parent()
        && !dirs.iter().any(|saved| saved == dir)
    {
        dirs.push(dir.to_path_buf());
    }
    dirs
}

fn find_binary(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    let executable = if cfg!(target_os = "windows") && !name.ends_with(".exe") {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    dirs.iter()
        .map(|dir| dir.join(&executable))
        .find(|candidate| candidate.is_file())
}

fn validate_output(path: &str) -> Result<(), ApiError> {
    let path = Path::new(path);
    if path.as_os_str().is_empty() {
        return Err(ApiError::invalid_input(
            "yt-dlp output directory is required",
        ));
    }
    if path.exists() && !path.is_dir() {
        return Err(ApiError::invalid_input(
            "yt-dlp output path is not a directory",
        ));
    }
    fs::create_dir_all(path).map_err(|error| {
        ApiError::internal(format!("could not create output directory: {error}"))
    })?;
    let probe = path.join(format!(".kopuz-write-test-{}", uuid::Uuid::new_v4()));
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .map_err(|error| {
            ApiError::invalid_input(format!("output directory is not writable: {error}"))
        })?;
    let _ = fs::remove_file(probe);
    Ok(())
}

fn add_options(command: &mut Command, options: &config::YtdlpOptions) {
    for (enabled, argument) in [
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
            command.arg(argument);
        }
    }
    if options.sponsorblock {
        command
            .arg("--sponsorblock-remove")
            .arg("sponsor,selfpromo,interaction");
    }
    if options.sponsorblock_mark {
        command
            .arg("--sponsorblock-mark")
            .arg("sponsor,selfpromo,interaction");
    }
    if options.postprocess_thumbnail_square {
        command
            .arg("--convert-thumbnails")
            .arg("png")
            .arg("--postprocessor-args")
            .arg(r#"ThumbnailsConvertor+FFmpeg_o:-c:v png -vf crop="'if(gt(ih,iw),iw,ih)':'if(gt(iw,ih),ih,iw)'""#);
    } else if !options.convert_thumbnail.trim().is_empty() {
        command
            .arg("--convert-thumbnails")
            .arg(options.convert_thumbnail.trim());
    }
    if !options.rate_limit.trim().is_empty() {
        command.arg("--limit-rate").arg(options.rate_limit.trim());
    }
    if !options.cookies_from_browser.trim().is_empty() {
        command
            .arg("--cookies-from-browser")
            .arg(options.cookies_from_browser.trim());
    }
    if !options.js_runtimes.trim().is_empty() {
        command.arg("--js-runtimes").arg(options.js_runtimes.trim());
    }
}

fn format_label(format: api::YtdlpAudioFormat) -> &'static str {
    match format {
        api::YtdlpAudioFormat::Best | api::YtdlpAudioFormat::Unknown => "Best Audio",
        api::YtdlpAudioFormat::Mp3 => "MP3",
        api::YtdlpAudioFormat::M4a => "M4A",
        api::YtdlpAudioFormat::Opus => "OPUS",
        api::YtdlpAudioFormat::Flac => "FLAC",
        api::YtdlpAudioFormat::Wav => "WAV",
        api::YtdlpAudioFormat::Video => "Video (MP4)",
    }
}

fn build_command(request: &api::YtdlpRequest) -> Result<Command, ApiError> {
    let dirs = search_dirs();
    let binary = find_binary("yt-dlp", &dirs)
        .ok_or_else(|| ApiError::not_found("yt-dlp was not found in PATH"))?;
    let ffmpeg = find_binary("ffmpeg", &dirs)
        .ok_or_else(|| ApiError::not_found("ffmpeg was not found in PATH"))?;
    let path = std::env::join_paths(&dirs)
        .map_err(|error| ApiError::internal(format!("could not construct PATH: {error}")))?;
    let options: config::YtdlpOptions = if request.options.is_null() {
        config::YtdlpOptions::default()
    } else {
        serde_json::from_value(request.options.clone())
            .map_err(|error| ApiError::invalid_input(format!("invalid yt-dlp options: {error}")))?
    };
    let mut command = Command::new(binary);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(0x0800_0000);
    }
    command
        .env("PATH", path)
        .current_dir(&request.output_dir)
        .arg("--ffmpeg-location")
        .arg(ffmpeg)
        .arg("--newline")
        .arg("--no-warnings")
        .arg("-o")
        .arg("%(album,playlist_title,title)s/%(uploader)s - %(title)s.%(ext)s")
        .arg("--paths")
        .arg(&request.output_dir);
    match request.format {
        api::YtdlpAudioFormat::Best | api::YtdlpAudioFormat::Unknown => {
            command.args(["-x", "--audio-quality", "0"]);
        }
        api::YtdlpAudioFormat::Mp3 => {
            command.args(["-x", "--audio-format", "mp3"]);
        }
        api::YtdlpAudioFormat::M4a => {
            command.args(["-x", "--audio-format", "m4a"]);
        }
        api::YtdlpAudioFormat::Opus => {
            command.args(["-x", "--audio-format", "opus"]);
        }
        api::YtdlpAudioFormat::Flac => {
            command.args(["-x", "--audio-format", "flac"]);
        }
        api::YtdlpAudioFormat::Wav => {
            command.args(["-x", "--audio-format", "wav"]);
        }
        api::YtdlpAudioFormat::Video => {
            command.args(["-f", "bestvideo+bestaudio", "--merge-output-format", "mp4"]);
        }
    }
    if request.format != api::YtdlpAudioFormat::Video {
        command
            .arg("--audio-quality")
            .arg(options.audio_quality.to_string());
    }
    add_options(&mut command, &options);
    command
        .arg(&request.url)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    Ok(command)
}

fn progress(line: &str) -> Option<(u64, String)> {
    if !line.starts_with("[download]") || !line.contains('%') {
        return None;
    }
    let percent = line
        .split('%')
        .next()?
        .split_whitespace()
        .last()?
        .parse::<f64>()
        .ok()?;
    Some(((percent.clamp(0.0, 100.0) * 100.0) as u64, line.to_string()))
}

fn run_process(mut command: Command, ctx: crate::jobs::JobCtx) -> Result<String, ApiError> {
    let mut child = command
        .spawn()
        .map_err(|error| ApiError::internal(format!("could not start yt-dlp: {error}")))?;
    let (tx, rx) = std::sync::mpsc::channel();
    if let Some(stdout) = child.stdout.take() {
        let tx = tx.clone();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout)
                .lines()
                .map_while(Result::ok)
            {
                let _ = tx.send(OutputLine::Line(line));
            }
        });
    }
    if let Some(stderr) = child.stderr.take() {
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(stderr)
                .lines()
                .map_while(Result::ok)
            {
                if line.contains("ERROR") {
                    let _ = tx.send(OutputLine::Stderr(line));
                }
            }
        });
    }
    let mut title = String::new();
    let mut errors = Vec::new();
    loop {
        if ctx.cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(title);
        }
        while let Ok(output) = rx.try_recv() {
            match output {
                OutputLine::Line(line) => {
                    if let Some((current, message)) = progress(&line) {
                        ctx.progress("downloading", Some(current), Some(10_000), Some(message));
                    } else if let Some(destination) = line.split("Destination:").nth(1) {
                        title = Path::new(destination.trim())
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or(destination.trim())
                            .to_string();
                    } else if line.contains("[ExtractAudio]") || line.contains("[ffmpeg]") {
                        ctx.progress("processing", Some(10_000), Some(10_000), None);
                    }
                }
                OutputLine::Stderr(line) => errors.push(line),
            }
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| ApiError::internal(format!("yt-dlp wait failed: {error}")))?
        {
            if status.success() {
                return Ok(title);
            }
            let message = if errors.is_empty() {
                format!("yt-dlp exited with {status}")
            } else {
                errors.join("\n")
            };
            return Err(ApiError::internal(message));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

pub fn spawn(
    runner: Arc<JobRunner>,
    config: Arc<ConfigService>,
    library: Arc<LibraryService>,
    session: SessionHandle,
    request: api::YtdlpRequest,
) -> Result<JobRef, ApiError> {
    if request.url.trim().is_empty() {
        return Err(ApiError::invalid_input("yt-dlp URL is required"));
    }
    validate_output(&request.output_dir)?;
    let scan_runner = runner.clone();
    runner.start(JobKind::Ytdlp, move |ctx| async move {
        let format = format_label(request.format).to_string();
        let url = request.url.clone();
        let command = build_command(&request)?;
        let task_ctx = ctx.clone();
        let result = tokio::task::spawn_blocking(move || run_process(command, task_ctx))
            .await
            .map_err(|error| ApiError::internal(format!("yt-dlp task failed: {error}")))?;
        let (status, error, title, failure) = match result {
            Ok(_) if ctx.cancelled() => return Ok(()),
            Ok(title) => ("completed".to_string(), None, title, None),
            Err(error) => (
                "failed".to_string(),
                Some(error.message.clone()),
                url.clone(),
                Some(error),
            ),
        };
        let history_error = error.clone();
        let updated = config
            .mutate_state(move |config| {
                config.ytdlp_history.insert(
                    0,
                    config::YtdlpHistoryEntry {
                        url,
                        title,
                        format,
                        status,
                        error: history_error.clone(),
                    },
                );
                config.ytdlp_history.truncate(200);
            })
            .await?;
        session.set_config(updated, vec!["ytdlp_history".to_string()]);
        if let Some(error) = failure {
            Err(error)
        } else {
            let _ = library.spawn_scan(&scan_runner);
            Ok(())
        }
    })
}
