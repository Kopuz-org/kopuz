//! Fetching a YouTube or YouTube Music link to files on disk.
//!
//! The link, the stream and the bytes all come through Kopuz's own YouTube
//! client ([`server::youtube_download`]), so an original-quality download needs
//! nothing installed. Tags and cover art are written by the same code the tag
//! editor uses; ffmpeg is only reached for to convert or to rewrap WebM audio.
//!
//! It runs as an ordinary job, so progress arrives on the event stream like
//! every other long-running task, and errors are codes rather than translated
//! strings: the client owns the locale. The options are published as a field
//! list the frontend renders without knowing what each one does.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use api::schema::{ChoiceOption, FieldKind, FieldSpec, FieldValue, Text, toggle_of, value_of};
use api::{ApiError, DownloadCandidate, DownloadHistoryEntry, DownloadState, JobKind, JobRef};
use server::youtube_download::{self, Link, YoutubeDownloader};
use server::ytmusic::YtStreamInfo;
use server::ytmusic::player::AudioFormat as StreamFormat;

use crate::config_service::ConfigService;
use crate::jobs::{JobCtx, JobRunner};

/// The one field that is not part of the stored options.
const OUTPUT_DIR: &str = "output_dir";

/// The settings keys behind the published rows.
const OUTPUT_DIR_KEY: &str = "ytdlp_output_dir";
const OPTIONS_KEY: &str = "ytdlp_options";
const HISTORY_KEY: &str = "ytdlp_history";

/// How many finished downloads the history keeps.
const HISTORY_LIMIT: usize = 50;

pub struct UrlDownloadService {
    config: Arc<ConfigService>,
    rescan: std::sync::OnceLock<(Arc<crate::library::LibraryService>, Arc<JobRunner>)>,
}

/// What one download was asked for, resolved from the caller's format and the
/// stored options.
struct Request {
    link: Link,
    output_dir: PathBuf,
    format: Format,
    options: config::DownloaderOptions,
    downloader: YoutubeDownloader,
    ffmpeg: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    /// The stream exactly as YouTube serves it, never re-encoded.
    BestAudio,
    Mp3,
    Flac,
    Opus,
    Wav,
}

impl Format {
    const ALL: [Self; 5] = [
        Self::BestAudio,
        Self::Mp3,
        Self::Flac,
        Self::Opus,
        Self::Wav,
    ];

    fn id(self) -> &'static str {
        match self {
            Self::BestAudio => "best_audio",
            Self::Mp3 => "mp3",
            Self::Flac => "flac",
            Self::Opus => "opus",
            Self::Wav => "wav",
        }
    }

    fn label(self) -> Text {
        Text::key(match self {
            Self::BestAudio => "downloader_format_best_audio",
            Self::Mp3 => "downloader_format_mp3",
            Self::Flac => "downloader_format_flac",
            Self::Opus => "downloader_format_opus",
            Self::Wav => "downloader_format_wav",
        })
    }

    fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|format| format.id() == id)
    }

    /// History written before formats had ids stored the label, and the
    /// yt-dlp downloader also offered video; those rows read back as the
    /// closest format still offered.
    fn from_stored(stored: &str) -> Self {
        match stored {
            "MP3" => Self::Mp3,
            "FLAC" => Self::Flac,
            "OPUS" => Self::Opus,
            "WAV" => Self::Wav,
            other => Self::from_id(other).unwrap_or(Self::BestAudio),
        }
    }

    /// Whether getting there from what YouTube serves takes an encoder.
    fn needs_encoder(self) -> bool {
        matches!(self, Self::Mp3 | Self::Flac | Self::Wav)
    }

    /// The file a download in this format ends up as, from a `source` stream.
    /// Opus inside WebM is rewrapped into Ogg when ffmpeg is there to do it,
    /// since only Ogg takes tags; without it the WebM is kept as Matroska,
    /// which it already is, so the library still scans it.
    fn target(self, source: StreamFormat, ffmpeg: bool) -> Target {
        match (self, source) {
            (Self::BestAudio, StreamFormat::M4a) => Target::Keep("m4a"),
            (Self::BestAudio | Self::Opus, StreamFormat::Webm) if ffmpeg => Target::Remux("opus"),
            (Self::BestAudio | Self::Opus, StreamFormat::Webm) => Target::Keep("mka"),
            (Self::Opus, StreamFormat::M4a) => {
                Target::Encode("opus", &["-c:a", "libopus", "-b:a", "160k"])
            }
            (Self::Mp3, _) => Target::Encode("mp3", &["-c:a", "libmp3lame", "-q:a", "0"]),
            (Self::Flac, _) => Target::Encode("flac", &["-c:a", "flac"]),
            (Self::Wav, _) => Target::Encode("wav", &["-c:a", "pcm_s16le"]),
        }
    }
}

/// How the fetched stream becomes the finished file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    /// Renamed into place as-is.
    Keep(&'static str),
    /// Copied into another container by ffmpeg, without re-encoding.
    Remux(&'static str),
    /// Re-encoded by ffmpeg with these codec arguments.
    Encode(&'static str, &'static [&'static str]),
}

impl Target {
    fn extension(self) -> &'static str {
        match self {
            Self::Keep(extension) | Self::Remux(extension) | Self::Encode(extension, _) => {
                extension
            }
        }
    }

    /// Lofty writes tags and pictures to these; a Matroska file it cannot.
    fn taggable(self) -> bool {
        self.extension() != "mka"
    }
}

/// Where to look for ffmpeg: the inherited PATH, the login shell's PATH, and
/// next to our own binary.
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
    let exe = if cfg!(target_os = "windows") {
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

/// The configured folder, else the system music folder, else home.
fn output_root(configured: &str) -> PathBuf {
    let configured = configured.trim();
    if !configured.is_empty() {
        return PathBuf::from(configured);
    }
    let dirs = directories::UserDirs::new();
    dirs.as_ref()
        .and_then(|dirs| dirs.audio_dir().map(Path::to_path_buf))
        .or_else(|| dirs.as_ref().map(|dirs| dirs.home_dir().to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// The output directory has to exist and be writable before the first byte
/// arrives, or the failure surfaces a hundred megabytes later.
fn prepare_output(path: &Path) -> Result<(), ApiError> {
    if path.exists() && !path.is_dir() {
        return Err(ApiError::invalid_input(
            "the download location is a file, not a folder",
        ));
    }
    std::fs::create_dir_all(path)
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

/// A file or folder name that is valid everywhere, from a title.
fn sanitize_component(value: &str) -> String {
    let sanitized: String = value
        .trim()
        .chars()
        .map(|character| match character {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            character if character.is_control() => '_',
            character => character,
        })
        .collect();
    let sanitized = sanitized.trim_matches([' ', '.']);
    if sanitized.is_empty() {
        "Untitled".to_string()
    } else {
        sanitized.chars().take(180).collect()
    }
}

/// "Artist - Title", or the title alone when there is no artist.
fn display_name(track: &reader::Track) -> String {
    if track.artist.trim().is_empty() {
        track.title.trim().to_string()
    } else {
        format!("{} - {}", track.artist.trim(), track.title.trim())
    }
}

/// `wanted`, or the first free "name (2).ext" beside it unless overwriting.
fn destination(wanted: PathBuf, overwrite: bool) -> PathBuf {
    if overwrite || !wanted.exists() {
        return wanted;
    }
    let parent = wanted.parent().unwrap_or_else(|| Path::new("."));
    let stem = wanted
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("download");
    let extension = wanted
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();
    (2..10_000)
        .map(|index| parent.join(format!("{stem} ({index}).{extension}")))
        .find(|candidate| !candidate.exists())
        .unwrap_or_else(|| parent.join(format!("{stem} {}.{extension}", uuid::Uuid::new_v4())))
}

/// YouTube Music covers come sized down for a list row; ask for a full one.
fn full_size_cover(url: &str) -> String {
    match url.rfind("=w") {
        Some(index) if url[index + 2..].starts_with(|c: char| c.is_ascii_digit()) => {
            format!("{}=w1200-h1200-l90-rj", &url[..index])
        }
        _ => url.to_string(),
    }
}

async fn fetch_cover(track: &reader::Track) -> Option<Vec<u8>> {
    let url = track
        .cover
        .as_deref()
        .filter(|url| url.starts_with("http"))?;
    let response = reqwest::get(full_size_cover(url)).await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.bytes().await.ok().map(|bytes| bytes.to_vec())
}

async fn run_ffmpeg(
    ffmpeg: &str,
    source: &Path,
    target: Target,
    output: &Path,
) -> Result<(), String> {
    let mut command = tokio::process::Command::new(ffmpeg);
    command
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i"])
        .arg(source)
        .args(["-map", "0:a:0", "-vn"]);
    match target {
        Target::Remux(_) => {
            command.args(["-c:a", "copy"]);
        }
        Target::Encode(_, codec) => {
            command.args(codec);
        }
        Target::Keep(_) => return Err("nothing for ffmpeg to do".to_string()),
    }
    command.arg(output).kill_on_drop(true);
    #[cfg(target_os = "windows")]
    command.creation_flags(0x0800_0000);

    let output = command
        .output()
        .await
        .map_err(|error| format!("ffmpeg would not start: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(if message.is_empty() {
        format!("ffmpeg exited with {}", output.status)
    } else {
        format!("ffmpeg: {message}")
    })
}

/// Tags and the front cover, through the same writer the tag editor uses.
async fn write_tags(
    path: &Path,
    track: &reader::Track,
    metadata: bool,
    cover: Option<Vec<u8>>,
) -> Result<(), String> {
    let edits = reader::TrackEdits {
        title: if metadata {
            track.title.clone()
        } else {
            String::new()
        },
        artist: if metadata {
            track.artist.clone()
        } else {
            String::new()
        },
        album: if metadata {
            track.album.clone()
        } else {
            String::new()
        },
        track_number: track.track_number.filter(|_| metadata),
        disc_number: track.disc_number.filter(|_| metadata),
        cover: cover.map_or(reader::CoverChange::Keep, reader::CoverChange::Set),
    };
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || reader::metadata::write_tags(&path, &edits))
        .await
        .map_err(|error| format!("tagging stopped: {error}"))?
}

impl UrlDownloadService {
    pub fn new(config: Arc<ConfigService>) -> Arc<Self> {
        Arc::new(Self {
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

    /// The formats a download can be asked for, the ones that take an encoder
    /// marked unavailable while ffmpeg is missing. Everything else about a
    /// download comes from the stored options.
    pub fn formats(&self) -> Vec<ChoiceOption> {
        let ffmpeg = find_binary("ffmpeg").is_some();
        Format::ALL
            .into_iter()
            .map(|format| ChoiceOption {
                value: format.id().to_string(),
                label: format.label(),
                unavailable: (format.needs_encoder() && !ffmpeg)
                    .then(|| Text::key("downloader_needs_ffmpeg")),
            })
            .collect()
    }

    pub async fn start(
        self: &Arc<Self>,
        runner: &JobRunner,
        url: String,
        format: String,
    ) -> Result<JobRef, ApiError> {
        let Some(link) = youtube_download::parse_link(&url) else {
            return Err(ApiError::invalid_input(
                "not a YouTube or YouTube Music link",
            ));
        };
        let Some(format) = Format::from_id(&format) else {
            return Err(ApiError::invalid_input("no such download format"));
        };
        let ffmpeg = find_binary("ffmpeg");
        if ffmpeg.is_none() && format.needs_encoder() {
            return Err(ApiError::unsupported("ffmpeg is not installed"));
        }
        let config = self.config.snapshot().await;
        let output_dir = output_root(&config.downloader_output_dir);
        prepare_output(&output_dir)?;
        let request = Request {
            link,
            output_dir,
            format,
            options: config.downloader_options.clone(),
            downloader: YoutubeDownloader::new(youtube_cookies(&config)),
            ffmpeg,
        };
        let service = self.clone();
        runner.start(JobKind::UrlDownload, move |ctx| async move {
            service.run(&ctx, request).await
        })
    }

    /// What `query` finds on YouTube Music, or the tracks it links to when it
    /// is a link, each with the URL [`Self::start`] takes to fetch just it.
    pub async fn search(&self, query: &str) -> Result<Vec<DownloadCandidate>, ApiError> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let config = self.config.snapshot().await;
        let downloader = YoutubeDownloader::new(youtube_cookies(&config));
        let tracks = match youtube_download::parse_link(query) {
            Some(link) => downloader.tracks(&link).await,
            None => downloader.search(query).await,
        }
        .map_err(ApiError::internal)?;
        Ok(tracks
            .iter()
            .map(|track| DownloadCandidate {
                url: Link::video_url(&track.id.key()),
                title: track.title.clone(),
                artist: track.artist.clone(),
                album: track.album.clone(),
                duration_secs: track.duration,
                cover_url: track.cover.clone().filter(|url| url.starts_with("http")),
            })
            .collect())
    }

    /// The downloader's options, with what they currently hold.
    pub async fn settings(&self) -> Vec<FieldSpec> {
        let config = self.config.snapshot().await;
        let options = &config.downloader_options;
        vec![
            FieldSpec {
                key: OUTPUT_DIR.to_string(),
                label: Text::key("downloader_output_dir_placeholder"),
                kind: FieldKind::Directory,
                value: Some(config.downloader_output_dir.clone()),
                config_key: Some(OUTPUT_DIR_KEY.to_string()),
                ..Default::default()
            },
            opens(
                "downloader_section_embed",
                toggle_field(
                    "embed_metadata",
                    "downloader_embed_metadata",
                    options.embed_metadata,
                ),
            ),
            toggle_field(
                "embed_thumbnail",
                "downloader_embed_thumbnail",
                options.embed_thumbnail,
            ),
            opens(
                "downloader_section_write",
                toggle_field(
                    "write_thumbnail",
                    "downloader_write_thumbnail",
                    options.write_thumbnail,
                ),
            ),
            opens(
                "downloader_section_behavior",
                toggle_field(
                    "organize_by_album",
                    "downloader_organize_by_album",
                    options.organize_by_album,
                ),
            ),
            toggle_field(
                "overwrite_existing",
                "downloader_overwrite_existing",
                options.overwrite_existing,
            ),
        ]
    }

    /// Answer the published options. An absent key is left alone.
    pub async fn set_settings(&self, values: Vec<FieldValue>) -> Result<Vec<FieldSpec>, ApiError> {
        let mut keys = Vec::new();
        if value_of(&values, OUTPUT_DIR).is_some() {
            keys.push(OUTPUT_DIR_KEY);
        }
        if values.iter().any(|value| value.key != OUTPUT_DIR) {
            keys.push(OPTIONS_KEY);
        }
        self.config.ensure_unlocked(&keys)?;
        self.config
            .mutate_state(&keys, move |config| {
                if let Some(dir) = value_of(&values, OUTPUT_DIR) {
                    config.downloader_output_dir = dir.trim().to_string();
                }
                apply_options(&values, &mut config.downloader_options);
            })
            .await?;
        Ok(self.settings().await)
    }

    pub async fn history(&self) -> Vec<DownloadHistoryEntry> {
        self.config
            .snapshot()
            .await
            .downloader_history
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
        self.config
            .mutate_state(&[HISTORY_KEY], |config| config.downloader_history.clear())
            .await?;
        Ok(())
    }

    /// Every track the link stands for, one after another. One failing does
    /// not stop the rest; the job fails at the end if any did.
    async fn run(&self, ctx: &JobCtx, request: Request) -> Result<(), ApiError> {
        ctx.progress("resolving", None, None, None);
        let tracks = request
            .downloader
            .tracks(&request.link)
            .await
            .map_err(ApiError::internal)?;
        let count = tracks.len() as u64;
        let mut failures = Vec::new();

        for (index, track) in tracks.iter().enumerate() {
            if ctx.cancelled() {
                break;
            }
            let label = if count > 1 {
                format!("{}/{count} · {}", index + 1, display_name(track))
            } else {
                display_name(track)
            };
            let step = Step {
                ctx,
                label: &label,
                done: index as u64,
                count,
            };
            let result = self.download_track(&request, track, &step).await;
            if let Err(error) = &result {
                tracing::warn!(video = %track.id.key(), %error, "a download failed");
                failures.push(error.clone());
            }
            self.record_history(track, request.format, result.err())
                .await;
        }

        if failures.len() < tracks.len()
            && let Some((library, jobs)) = self.rescan.get()
            && let Err(error) = library.spawn_scan(jobs)
        {
            tracing::debug!(%error, "no rescan after the download");
        }
        match failures.as_slice() {
            [] => Ok(()),
            [only] if count == 1 => Err(ApiError::internal(only.clone())),
            _ => Err(ApiError::internal(format!(
                "{} of {count} downloads failed",
                failures.len()
            ))),
        }
    }

    async fn download_track(
        &self,
        request: &Request,
        track: &reader::Track,
        step: &Step<'_>,
    ) -> Result<PathBuf, String> {
        let video_id = track.id.key();
        let stream = request.downloader.stream(&video_id).await?;
        let target = request
            .format
            .target(stream.format, request.ffmpeg.is_some());

        let folder = match request.options.organize_by_album && !track.album.trim().is_empty() {
            true => request.output_dir.join(sanitize_component(&track.album)),
            false => request.output_dir.clone(),
        };
        tokio::fs::create_dir_all(&folder)
            .await
            .map_err(|error| format!("cannot create {}: {error}", folder.display()))?;
        let stem = sanitize_component(&display_name(track));
        let wanted = folder.join(format!("{stem}.{}", target.extension()));
        let output = destination(wanted, request.options.overwrite_existing);
        let partial = folder.join(format!(".{}.part", uuid::Uuid::new_v4()));

        let result = self
            .produce(request, track, &stream, target, &partial, &output, step)
            .await;
        let _ = tokio::fs::remove_file(&partial).await;
        if result.is_err() {
            let _ = tokio::fs::remove_file(&output).await;
        }
        result.map(|()| output)
    }

    /// Fetch, convert if asked, then tag, landing on `output`.
    #[allow(clippy::too_many_arguments)]
    async fn produce(
        &self,
        request: &Request,
        track: &reader::Track,
        stream: &YtStreamInfo,
        target: Target,
        partial: &Path,
        output: &Path,
        step: &Step<'_>,
    ) -> Result<(), String> {
        step.report("downloading", 0);
        let cancelled = || step.ctx.cancelled();
        let mut progress = |written: u64, total: Option<u64>| {
            if let Some(total) = total.filter(|total| *total > 0) {
                step.report("downloading", written * 100 / total);
            }
        };
        youtube_download::fetch_stream(stream, partial, &cancelled, &mut progress).await?;

        let options = &request.options;
        let cover = match options.embed_thumbnail || options.write_thumbnail {
            true => fetch_cover(track).await,
            false => None,
        };

        step.report("processing", 100);
        match (target, request.ffmpeg.as_deref()) {
            (Target::Keep(_), _) => tokio::fs::rename(partial, output)
                .await
                .map_err(|error| format!("cannot move the download into place: {error}"))?,
            (_, Some(ffmpeg)) => run_ffmpeg(ffmpeg, partial, target, output).await?,
            (_, None) => return Err("ffmpeg is not installed".to_string()),
        }

        let embedded_cover = cover.clone().filter(|_| options.embed_thumbnail);
        if target.taggable()
            && (options.embed_metadata || embedded_cover.is_some())
            && let Err(error) =
                write_tags(output, track, options.embed_metadata, embedded_cover).await
        {
            tracing::warn!(%error, path = %output.display(), "the download kept no tags");
        }
        let sidecar_wanted =
            options.write_thumbnail || (options.embed_thumbnail && !target.taggable());
        let sidecar = output.with_extension("jpg");
        if let Some(cover) = cover.filter(|_| sidecar_wanted)
            && (options.overwrite_existing || !sidecar.exists())
            && let Err(error) = tokio::fs::write(&sidecar, cover).await
        {
            tracing::warn!(%error, "the cover could not be saved beside the download");
        }
        Ok(())
    }

    /// Downloads are remembered so the page can show what happened after a
    /// restart, which is why this is config rather than a job list.
    async fn record_history(&self, track: &reader::Track, format: Format, error: Option<String>) {
        let entry = config::DownloaderHistoryEntry {
            url: Link::video_url(&track.id.key()),
            title: display_name(track),
            format: format.id().to_string(),
            status: if error.is_some() {
                "failed".to_string()
            } else {
                "completed".to_string()
            },
            error,
        };
        if let Err(error) = self
            .config
            .mutate_state(&[HISTORY_KEY], move |config| {
                config.downloader_history.insert(0, entry);
                config.downloader_history.truncate(HISTORY_LIMIT);
            })
            .await
        {
            tracing::warn!(%error, "the download history could not be saved");
        }
    }
}

/// Where one track sits in the job, so its progress reads as part of the whole.
struct Step<'a> {
    ctx: &'a JobCtx,
    label: &'a str,
    done: u64,
    count: u64,
}

impl Step<'_> {
    fn report(&self, phase: &str, percent: u64) {
        self.ctx.progress_throttled(
            phase,
            Some(self.done * 100 + percent.min(100)),
            Some(self.count * 100),
            Some(self.label.to_string()),
        );
    }
}

/// The signed-in YouTube Music session's cookies, when that is the configured
/// server, so Premium streams download at Premium quality.
fn youtube_cookies(config: &config::AppConfig) -> Option<String> {
    config
        .server
        .as_ref()
        .and_then(|server| server.ytmusic_cookies())
        .map(str::to_string)
}

fn toggle_field(key: &str, label: &str, on: bool) -> FieldSpec {
    FieldSpec {
        key: key.to_string(),
        label: Text::key(label),
        kind: FieldKind::Toggle,
        value: Some(on.to_string()),
        config_key: Some(OPTIONS_KEY.to_string()),
        ..Default::default()
    }
}

/// Start a titled group before this row.
fn opens(section: &str, mut field: FieldSpec) -> FieldSpec {
    field.section = Some(Text::key(section));
    field
}

/// Fold answered options into the stored ones.
fn apply_options(values: &[FieldValue], options: &mut config::DownloaderOptions) {
    for (key, current) in [
        ("embed_metadata", &mut options.embed_metadata),
        ("embed_thumbnail", &mut options.embed_thumbnail),
        ("write_thumbnail", &mut options.write_thumbnail),
        ("organize_by_album", &mut options.organize_by_album),
        ("overwrite_existing", &mut options.overwrite_existing),
    ] {
        *current = toggle_of(values, key, *current);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_anonymous_source_downloads_without_a_leftover_token() {
        let mut server = config::MusicServer::new_with_service(
            "YouTube Music".into(),
            "https://music.youtube.com".into(),
            config::MusicService::YtMusic,
        );
        server.access_token = Some("SAPISID=leftover".into());
        server.yt_anonymous = true;
        let mut config = config::AppConfig {
            server: Some(server),
            ..Default::default()
        };
        assert_eq!(youtube_cookies(&config), None);
        if let Some(server) = config.server.as_mut() {
            server.yt_anonymous = false;
        }
        assert_eq!(
            youtube_cookies(&config).as_deref(),
            Some("SAPISID=leftover")
        );
    }

    /// History written before the rename stored the label, and the yt-dlp
    /// downloader's video rows are still on disk.
    #[test]
    fn a_stored_format_reads_back_as_an_option_id() {
        assert_eq!(Format::from_stored("Video (MP4)").id(), "best_audio");
        assert_eq!(Format::from_stored("video").id(), "best_audio");
        assert_eq!(Format::from_stored("MP3").id(), "mp3");
        assert_eq!(Format::from_stored("flac").id(), "flac");
        assert_eq!(Format::from_stored("nonsense").id(), "best_audio");
    }

    /// Only encoding needs ffmpeg; the original stream never does, and an Opus
    /// stream asked for as Opus is copied rather than re-encoded.
    #[test]
    fn ffmpeg_is_only_needed_to_convert() {
        assert_eq!(
            Format::BestAudio.target(StreamFormat::M4a, false),
            Target::Keep("m4a")
        );
        assert_eq!(
            Format::BestAudio.target(StreamFormat::Webm, false),
            Target::Keep("mka")
        );
        assert_eq!(
            Format::Opus.target(StreamFormat::Webm, true),
            Target::Remux("opus")
        );
        assert!(matches!(
            Format::Mp3.target(StreamFormat::Webm, true),
            Target::Encode("mp3", _)
        ));
        assert!(!Format::BestAudio.needs_encoder());
        assert!(!Format::Opus.needs_encoder());
        assert!(Format::Flac.needs_encoder());
    }

    #[test]
    fn names_are_safe_on_every_filesystem() {
        assert_eq!(sanitize_component("AC/DC: Live?"), "AC_DC_ Live_");
        assert_eq!(sanitize_component("  ...  "), "Untitled");
    }

    #[test]
    fn a_cover_is_asked_for_at_full_size() {
        assert_eq!(
            full_size_cover("https://lh3.googleusercontent.com/abc=w544-h544-l90-rj"),
            "https://lh3.googleusercontent.com/abc=w1200-h1200-l90-rj"
        );
        assert_eq!(
            full_size_cover("https://i.ytimg.com/vi/x/maxresdefault.jpg"),
            "https://i.ytimg.com/vi/x/maxresdefault.jpg"
        );
    }
}
