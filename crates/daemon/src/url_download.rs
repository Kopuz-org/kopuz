//! Fetching a URL to a file, with yt-dlp.
//!
//! A subprocess, a PATH search, an ffmpeg lookup and a filesystem write --
//! system-level work that ran in the UI process, which meant navigating away
//! could kill a download and no other frontend could start or watch one.
//!
//! It runs as an ordinary job here, so progress arrives on the event stream
//! like every other long-running task, and errors are codes rather than
//! translated strings: the client owns the locale.
//!
//! The options are published as a field list too. A frontend picks a format
//! and renders the rows it is given; which yt-dlp flag each one turns into is
//! this file's business.

use std::io::BufRead as _;
use std::path::PathBuf;
use std::sync::Arc;

use api::schema::{ChoiceOption, FieldKind, FieldSpec, FieldValue, Text, toggle_of, value_of};
use api::{ApiError, DownloadHistoryEntry, DownloadState, JobKind, JobRef};

use crate::config_service::ConfigService;
use crate::jobs::{JobCtx, JobRunner};
use crate::session::SessionHandle;

/// The one field that is not part of `ytdlp_options`.
const OUTPUT_DIR: &str = "output_dir";

/// The settings keys behind the published rows.
const OUTPUT_DIR_KEY: &str = "ytdlp_output_dir";
const OPTIONS_KEY: &str = "ytdlp_options";
const HISTORY_KEY: &str = "ytdlp_history";

/// How many finished downloads the history keeps.
const HISTORY_LIMIT: usize = 50;

pub struct UrlDownloadService {
    session: SessionHandle,
    config: Arc<ConfigService>,
    rescan: std::sync::OnceLock<(Arc<crate::library::LibraryService>, Arc<JobRunner>)>,
}

/// What one download was asked for, resolved from the caller's format and the
/// stored options.
struct Request {
    url: String,
    output_dir: String,
    format: Format,
    options: config::YtdlpOptions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    BestAudio,
    Mp3,
    Flac,
    Opus,
    Wav,
    Video,
}

impl Format {
    const ALL: [Self; 6] = [
        Self::BestAudio,
        Self::Mp3,
        Self::Flac,
        Self::Opus,
        Self::Wav,
        Self::Video,
    ];

    fn id(self) -> &'static str {
        match self {
            Self::BestAudio => "best_audio",
            Self::Mp3 => "mp3",
            Self::Flac => "flac",
            Self::Opus => "opus",
            Self::Wav => "wav",
            Self::Video => "video",
        }
    }

    fn label(self) -> Text {
        Text::key(match self {
            Self::BestAudio => "ytdlp_format_best_audio",
            Self::Mp3 => "ytdlp_format_mp3",
            Self::Flac => "ytdlp_format_flac",
            Self::Opus => "ytdlp_format_opus",
            Self::Wav => "ytdlp_format_wav",
            Self::Video => "ytdlp_format_video",
        })
    }

    fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|format| format.id() == id)
    }

    /// A history row from before formats had ids stored the label instead, so
    /// those are read back as the format they named.
    fn from_stored(stored: &str) -> Self {
        match stored {
            "MP3" => Self::Mp3,
            "FLAC" => Self::Flac,
            "OPUS" => Self::Opus,
            "WAV" => Self::Wav,
            "Video (MP4)" => Self::Video,
            other => Self::from_id(other).unwrap_or(Self::BestAudio),
        }
    }

    fn args(self) -> Vec<&'static str> {
        match self {
            Self::BestAudio => vec!["-x", "--audio-quality", "0"],
            Self::Mp3 => vec!["-x", "--audio-format", "mp3", "--audio-quality", "0"],
            Self::Flac => vec!["-x", "--audio-format", "flac"],
            Self::Opus => vec!["-x", "--audio-format", "opus"],
            Self::Wav => vec!["-x", "--audio-format", "wav"],
            Self::Video => vec!["-f", "bestvideo+bestaudio", "--merge-output-format", "mp4"],
        }
    }
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

fn build_command(request: &Request) -> std::process::Command {
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
    for arg in request.format.args() {
        cmd.arg(arg);
    }

    let options = &request.options;
    if request.format != Format::Video {
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

/// One published option, named by its `YtdlpOptions` field and labelled by the
/// flag it becomes.
fn option_field(key: &str, label: &str, flag: &str, kind: FieldKind, value: String) -> FieldSpec {
    FieldSpec {
        key: key.to_string(),
        label: Text::key(label),
        help: Some(Text::literal(flag)),
        kind,
        value: Some(value),
        config_key: Some(OPTIONS_KEY.to_string()),
        ..Default::default()
    }
}

/// The rows the page labels with the flag itself, having no wording of their
/// own to translate.
fn flag_field(key: &str, flag: &str, kind: FieldKind, value: String) -> FieldSpec {
    FieldSpec {
        key: key.to_string(),
        label: Text::literal(flag),
        kind,
        value: Some(value),
        config_key: Some(OPTIONS_KEY.to_string()),
        ..Default::default()
    }
}

fn toggle_field(key: &str, label: &str, flag: &str, on: bool) -> FieldSpec {
    option_field(key, label, flag, FieldKind::Toggle, on.to_string())
}

/// Start a titled group before this row.
fn opens(section: &str, mut field: FieldSpec) -> FieldSpec {
    field.section = Some(Text::key(section));
    field
}

fn choice(values: &[(&str, Text)]) -> FieldKind {
    FieldKind::Choice {
        options: values
            .iter()
            .map(|(value, label)| ChoiceOption {
                value: (*value).to_string(),
                label: label.clone(),
            })
            .collect(),
        custom: false,
    }
}

fn thumbnail_formats() -> FieldKind {
    choice(&[
        ("", Text::key("ytdlp_none")),
        ("jpg", Text::literal("JPG")),
        ("png", Text::literal("PNG")),
        ("webp", Text::literal("WebP")),
    ])
}

fn cookie_browsers() -> FieldKind {
    choice(&[
        ("", Text::key("ytdlp_none")),
        ("chrome", Text::literal("Chrome")),
        ("firefox", Text::literal("Firefox")),
        ("chromium", Text::literal("Chromium")),
        ("edge", Text::literal("Edge")),
        ("safari", Text::literal("Safari")),
        ("brave", Text::literal("Brave")),
        ("vivaldi", Text::literal("Vivaldi")),
    ])
}

/// yt-dlp's own scale: 0 is the best it can do, 10 the smallest file.
fn audio_qualities() -> FieldKind {
    FieldKind::Choice {
        options: (0..=10)
            .map(|level| ChoiceOption {
                value: level.to_string(),
                label: Text::literal(level.to_string()),
            })
            .collect(),
        custom: false,
    }
}

impl UrlDownloadService {
    pub fn new(session: SessionHandle, config: Arc<ConfigService>) -> Arc<Self> {
        Arc::new(Self {
            session,
            config,
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

    /// The formats a download can be asked for. Everything else about it comes
    /// from the stored options.
    pub fn formats(&self) -> Vec<ChoiceOption> {
        Format::ALL
            .into_iter()
            .map(|format| ChoiceOption {
                value: format.id().to_string(),
                label: format.label(),
            })
            .collect()
    }

    pub async fn start(
        self: &Arc<Self>,
        runner: &JobRunner,
        url: String,
        format: String,
    ) -> Result<JobRef, ApiError> {
        let url = url.trim().to_string();
        if url.is_empty() {
            return Err(ApiError::invalid_input("a download needs a URL"));
        }
        let Some(format) = Format::from_id(&format) else {
            return Err(ApiError::invalid_input("no such download format"));
        };
        if find_binary("yt-dlp").is_none() {
            return Err(ApiError::unsupported("yt-dlp is not installed"));
        }
        if find_binary("ffmpeg").is_none() {
            return Err(ApiError::unsupported("ffmpeg is not installed"));
        }
        let config = self.config.snapshot().await;
        let request = Request {
            url,
            output_dir: config.ytdlp_output_dir.clone(),
            format,
            options: config.ytdlp_options.clone(),
        };
        prepare_output(&request.output_dir)?;
        let service = self.clone();
        runner.start(JobKind::UrlDownload, move |ctx| async move {
            service.run(&ctx, request).await
        })
    }

    /// The downloader's options, with what they currently hold.
    pub async fn settings(&self) -> Vec<FieldSpec> {
        let config = self.config.snapshot().await;
        let options = &config.ytdlp_options;
        vec![
            FieldSpec {
                key: OUTPUT_DIR.to_string(),
                label: Text::key("ytdlp_output_dir_placeholder"),
                kind: FieldKind::Directory,
                value: Some(config.ytdlp_output_dir.clone()),
                config_key: Some(OUTPUT_DIR_KEY.to_string()),
                ..Default::default()
            },
            opens(
                "ytdlp_section_embed",
                toggle_field(
                    "embed_metadata",
                    "ytdlp_embed_metadata",
                    "--embed-metadata",
                    options.embed_metadata,
                ),
            ),
            toggle_field(
                "embed_thumbnail",
                "ytdlp_embed_thumbnail",
                "--embed-thumbnail",
                options.embed_thumbnail,
            ),
            toggle_field(
                "embed_chapters",
                "ytdlp_embed_chapters",
                "--embed-chapters",
                options.embed_chapters,
            ),
            toggle_field(
                "embed_subs",
                "ytdlp_embed_subtitles",
                "--embed-subs",
                options.embed_subs,
            ),
            toggle_field(
                "embed_info_json",
                "ytdlp_embed_info_json",
                "--embed-info-json",
                options.embed_info_json,
            ),
            opens(
                "ytdlp_section_write",
                toggle_field(
                    "write_thumbnail",
                    "ytdlp_write_thumbnail",
                    "--write-thumbnail",
                    options.write_thumbnail,
                ),
            ),
            toggle_field(
                "write_description",
                "ytdlp_write_description",
                "--write-description",
                options.write_description,
            ),
            toggle_field(
                "write_info_json",
                "ytdlp_write_info_json",
                "--write-info-json",
                options.write_info_json,
            ),
            toggle_field(
                "write_subs",
                "ytdlp_write_subtitles",
                "--write-subs",
                options.write_subs,
            ),
            toggle_field(
                "write_auto_subs",
                "ytdlp_write_auto_subtitles",
                "--write-auto-subs",
                options.write_auto_subs,
            ),
            toggle_field(
                "write_comments",
                "ytdlp_write_comments",
                "--write-comments",
                options.write_comments,
            ),
            opens(
                "ytdlp_section_postprocess",
                toggle_field(
                    "sponsorblock",
                    "ytdlp_remove_sponsors",
                    "--sponsorblock-remove",
                    options.sponsorblock,
                ),
            ),
            toggle_field(
                "sponsorblock_mark",
                "ytdlp_mark_sponsors",
                "--sponsorblock-mark",
                options.sponsorblock_mark,
            ),
            toggle_field(
                "split_chapters",
                "ytdlp_split_chapters",
                "--split-chapters",
                options.split_chapters,
            ),
            toggle_field(
                "postprocess_thumbnail_square",
                "ytdlp_crop_thumbnails",
                "--postprocessor-args",
                options.postprocess_thumbnail_square,
            ),
            flag_field(
                "convert_thumbnail",
                "--convert-thumbnails",
                thumbnail_formats(),
                options.convert_thumbnail.clone(),
            ),
            flag_field(
                "audio_quality",
                "--audio-quality",
                audio_qualities(),
                options.audio_quality.to_string(),
            ),
            opens(
                "ytdlp_section_behavior",
                toggle_field(
                    "no_playlist",
                    "ytdlp_single_video",
                    "--no-playlist",
                    options.no_playlist,
                ),
            ),
            toggle_field("xattrs", "ytdlp_write_xattrs", "--xattrs", options.xattrs),
            toggle_field("no_mtime", "ytdlp_no_mtime", "--no-mtime", options.no_mtime),
            FieldSpec {
                placeholder: Some(Text::key("ytdlp_unlimited")),
                help: Some(Text::literal("e.g. 1M, 500K")),
                ..flag_field(
                    "rate_limit",
                    "--limit-rate",
                    FieldKind::Text,
                    options.rate_limit.clone(),
                )
            },
            flag_field(
                "cookies_from_browser",
                "--cookies-from-browser",
                cookie_browsers(),
                options.cookies_from_browser.clone(),
            ),
            FieldSpec {
                placeholder: Some(Text::literal("deno, node, bun or quickjs[:/path]")),
                help: Some(Text::key("ytdlp_js_runtimes_tooltip")),
                ..flag_field(
                    "js_runtimes",
                    "--js-runtimes",
                    FieldKind::Text,
                    options.js_runtimes.clone(),
                )
            },
        ]
    }

    /// Answer the published options. An absent key is left alone.
    pub async fn set_settings(&self, values: Vec<FieldValue>) -> Result<Vec<FieldSpec>, ApiError> {
        let mut keys = Vec::new();
        if value_of(&values, OUTPUT_DIR).is_some() {
            keys.push(OUTPUT_DIR_KEY);
        }
        // Every other row is a field of the one `ytdlp_options` key.
        if values.iter().any(|value| value.key != OUTPUT_DIR) {
            keys.push(OPTIONS_KEY);
        }
        self.config.ensure_unlocked(&keys)?;
        let updated = self
            .config
            .mutate_state(move |config| {
                if let Some(dir) = value_of(&values, OUTPUT_DIR) {
                    config.ytdlp_output_dir = dir.trim().to_string();
                }
                apply_options(&values, &mut config.ytdlp_options);
            })
            .await?;
        self.session
            .set_config(updated, keys.iter().map(|key| (*key).to_string()).collect());
        Ok(self.settings().await)
    }

    pub async fn history(&self) -> Vec<DownloadHistoryEntry> {
        self.config
            .snapshot()
            .await
            .ytdlp_history
            .iter()
            .map(|entry| DownloadHistoryEntry {
                url: entry.url.clone(),
                title: entry.title.clone(),
                format: Format::from_stored(&entry.format).id().to_string(),
                state: match entry.status.as_str() {
                    "completed" => DownloadState::Finished,
                    _ => DownloadState::Failed,
                },
                error: entry.error.clone(),
            })
            .collect()
    }

    pub async fn clear_history(&self) -> Result<(), ApiError> {
        self.config.ensure_unlocked(&[HISTORY_KEY])?;
        let updated = self
            .config
            .mutate_state(|config| config.ytdlp_history.clear())
            .await?;
        self.publish_history(updated);
        Ok(())
    }

    async fn run(&self, ctx: &JobCtx, request: Request) -> Result<(), ApiError> {
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
    async fn record_history(&self, url: &str, title: &str, format: Format, error: Option<String>) {
        let entry = config::YtdlpHistoryEntry {
            url: url.to_string(),
            title: title.to_string(),
            format: format.id().to_string(),
            status: if error.is_some() {
                "failed".to_string()
            } else {
                "completed".to_string()
            },
            error,
        };
        match self
            .config
            .mutate_state(move |config| {
                config.ytdlp_history.insert(0, entry);
                config.ytdlp_history.truncate(HISTORY_LIMIT);
            })
            .await
        {
            Ok(updated) => self.publish_history(updated),
            Err(error) => tracing::warn!(%error, "the download history could not be saved"),
        }
    }

    fn publish_history(&self, updated: config::AppConfig) {
        self.session
            .set_config(updated, vec![HISTORY_KEY.to_string()]);
    }
}

/// Fold answered options into the stored ones.
fn apply_options(values: &[FieldValue], options: &mut config::YtdlpOptions) {
    for (key, current) in [
        ("embed_metadata", &mut options.embed_metadata),
        ("embed_thumbnail", &mut options.embed_thumbnail),
        ("embed_chapters", &mut options.embed_chapters),
        ("embed_subs", &mut options.embed_subs),
        ("embed_info_json", &mut options.embed_info_json),
        ("write_thumbnail", &mut options.write_thumbnail),
        ("write_description", &mut options.write_description),
        ("write_info_json", &mut options.write_info_json),
        ("write_subs", &mut options.write_subs),
        ("write_auto_subs", &mut options.write_auto_subs),
        ("write_comments", &mut options.write_comments),
        ("sponsorblock", &mut options.sponsorblock),
        ("sponsorblock_mark", &mut options.sponsorblock_mark),
        ("split_chapters", &mut options.split_chapters),
        (
            "postprocess_thumbnail_square",
            &mut options.postprocess_thumbnail_square,
        ),
        ("no_playlist", &mut options.no_playlist),
        ("xattrs", &mut options.xattrs),
        ("no_mtime", &mut options.no_mtime),
    ] {
        *current = toggle_of(values, key, *current);
    }
    for (key, current) in [
        ("convert_thumbnail", &mut options.convert_thumbnail),
        ("rate_limit", &mut options.rate_limit),
        ("cookies_from_browser", &mut options.cookies_from_browser),
        ("js_runtimes", &mut options.js_runtimes),
    ] {
        if let Some(value) = value_of(values, key) {
            *current = value.to_string();
        }
    }
    if let Some(quality) = value_of(values, "audio_quality").and_then(|value| value.parse().ok()) {
        options.audio_quality = quality;
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

    /// History written before the rename stored the label, and those rows are
    /// still on disk.
    #[test]
    fn a_stored_format_reads_back_as_an_option_id() {
        assert_eq!(Format::from_stored("Video (MP4)").id(), "video");
        assert_eq!(Format::from_stored("MP3").id(), "mp3");
        assert_eq!(Format::from_stored("flac").id(), "flac");
        assert_eq!(Format::from_stored("nonsense").id(), "best_audio");
    }
}
