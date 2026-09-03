use config::{AppConfig, YoutubeDownloadHistoryEntry, YoutubeDownloadOptions};
use dioxus::core::spawn_forever;
use dioxus::prelude::*;
use reader::{CoverRef, Track};
use server::youtube_download::YoutubeDownloadClient;
use server::ytmusic::player::AudioFormat as StreamFormat;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::Instant;
use tokio::io::AsyncWriteExt;

/// App-lifetime jobs continue while the user visits another page.
pub(crate) static JOBS: GlobalSignal<Vec<DownloadJob>> = Signal::global(Vec::new);

/// Completions are applied by a hook in the app scope because detached job
/// drivers must not retain page-scoped config signals.
static FINISHED: GlobalSignal<Vec<(YoutubeDownloadHistoryEntry, bool)>> = Signal::global(Vec::new);

pub fn use_youtube_download_completion_sink(
    mut config: Signal<AppConfig>,
    mut trigger_rescan: Signal<usize>,
) {
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
                cfg.youtube_download_history.insert(0, entry);
                cfg.youtube_download_history.truncate(200);
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
    pub video_id: String,
    pub title: String,
    pub artist: String,
    pub format: AudioFormat,
    pub progress: f64,
    pub status: JobStatus,
    pub speed: String,
    pub eta: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum JobStatus {
    Pending,
    Resolving,
    Downloading,
    Processing,
    Completed,
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Copy)]
pub enum AudioFormat {
    Original,
    Mp3,
    Flac,
    Opus,
    Wav,
}

impl AudioFormat {
    fn label_key(self) -> &'static str {
        match self {
            Self::Original => "youtube_download_format_original",
            Self::Mp3 => "youtube_download_format_mp3",
            Self::Flac => "youtube_download_format_flac",
            Self::Opus => "youtube_download_format_opus",
            Self::Wav => "youtube_download_format_wav",
        }
    }

    pub fn label(self) -> String {
        i18n::t(self.label_key())
    }

    fn storage_label(self) -> &'static str {
        match self {
            Self::Original => "Original",
            Self::Mp3 => "MP3",
            Self::Flac => "FLAC",
            Self::Opus => "OPUS",
            Self::Wav => "WAV",
        }
    }

    fn from_str(value: &str) -> Self {
        match value {
            "MP3" => Self::Mp3,
            "FLAC" => Self::Flac,
            "OPUS" => Self::Opus,
            "WAV" => Self::Wav,
            _ => Self::Original,
        }
    }

    fn needs_conversion(self) -> bool {
        !matches!(self, Self::Original)
    }
}

pub fn seed_from_history(history: &[YoutubeDownloadHistoryEntry]) {
    if !JOBS.read().is_empty() {
        return;
    }

    *JOBS.write() = history
        .iter()
        .map(|entry| DownloadJob {
            id: uuid::Uuid::new_v4().to_string(),
            video_id: entry.video_id.clone(),
            title: entry.title.clone(),
            artist: String::new(),
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
            JobStatus::Pending
                | JobStatus::Resolving
                | JobStatus::Downloading
                | JobStatus::Processing
        )
    });
}

pub fn run_preflight_checks(
    video_id: &str,
    out_dir: &str,
    format: AudioFormat,
    options: &YoutubeDownloadOptions,
) -> Result<(), String> {
    if JOBS.read().iter().any(|job| {
        job.video_id == video_id
            && matches!(
                job.status,
                JobStatus::Pending
                    | JobStatus::Resolving
                    | JobStatus::Downloading
                    | JobStatus::Processing
            )
    }) {
        return Err(i18n::t("youtube_download_error_duplicate_active"));
    }

    if requires_ffmpeg(format, options) && find_binary("ffmpeg").is_none() {
        return Err(i18n::t("youtube_download_error_ffmpeg_required"));
    }

    validate_output_directory(&output_root(out_dir))
}

pub fn start_download(
    track: Track,
    client: YoutubeDownloadClient,
    out_dir: String,
    format: AudioFormat,
    options: YoutubeDownloadOptions,
) {
    let video_id = track.id.key().into_owned();
    let job_id = uuid::Uuid::new_v4().to_string();

    JOBS.write().insert(
        0,
        DownloadJob {
            id: job_id.clone(),
            video_id: video_id.clone(),
            title: track.title.clone(),
            artist: track.artist.clone(),
            format,
            progress: 0.0,
            status: JobStatus::Pending,
            speed: String::new(),
            eta: String::new(),
        },
    );

    spawn_forever(async move {
        set_status(&job_id, JobStatus::Resolving);
        let result = download_track(&job_id, &track, &client, &out_dir, format, &options).await;

        match result {
            Ok(path) => {
                tracing::info!(
                    target: "youtube_download",
                    video_id,
                    path = %path.display(),
                    "native YouTube download finished"
                );
                if let Some(job) = JOBS.write().iter_mut().find(|job| job.id == job_id) {
                    job.status = JobStatus::Completed;
                    job.progress = 100.0;
                    job.speed.clear();
                    job.eta.clear();
                }
                FINISHED.write().push((history_entry(&job_id, None), true));
            }
            Err(error) => {
                tracing::error!(
                    target: "youtube_download",
                    video_id,
                    error,
                    "native YouTube download failed"
                );
                let entry = history_entry(&job_id, Some(error.clone()));
                set_status(&job_id, JobStatus::Failed(error));
                FINISHED.write().push((entry, false));
            }
        }
    });
}

fn history_entry(job_id: &str, error: Option<String>) -> YoutubeDownloadHistoryEntry {
    let job = JOBS.read().iter().find(|job| job.id == job_id).cloned();
    match job {
        Some(job) => YoutubeDownloadHistoryEntry {
            video_id: job.video_id,
            title: job.title,
            format: job.format.storage_label().to_string(),
            status: if error.is_some() {
                "failed".to_string()
            } else {
                "completed".to_string()
            },
            error,
        },
        None => YoutubeDownloadHistoryEntry {
            video_id: String::new(),
            title: String::new(),
            format: AudioFormat::Original.storage_label().to_string(),
            status: "failed".to_string(),
            error,
        },
    }
}

async fn download_track(
    job_id: &str,
    track: &Track,
    client: &YoutubeDownloadClient,
    out_dir: &str,
    format: AudioFormat,
    options: &YoutubeDownloadOptions,
) -> Result<PathBuf, String> {
    let stream = client.resolve_stream(&track.id.key()).await?;
    let root = output_root(out_dir);
    let destination_dir = if options.organize_by_album && !track.album.trim().is_empty() {
        root.join(sanitize_component(&track.album))
    } else {
        root
    };
    tokio::fs::create_dir_all(&destination_dir)
        .await
        .map_err(|error| format!("failed to create output directory: {error}"))?;

    let stem = output_stem(track);
    let process = requires_ffmpeg(format, options);
    let extension = output_extension(format, stream.format, process);
    let wanted = destination_dir.join(format!("{stem}.{extension}"));
    let destination = destination_path(&wanted, options.overwrite_existing);
    let source_path = destination_dir.join(format!(
        ".{}.source.{}",
        uuid::Uuid::new_v4(),
        stream.format.extension()
    ));

    set_status(job_id, JobStatus::Downloading);
    if let Err(error) = download_stream(job_id, &stream, &source_path).await {
        let _ = tokio::fs::remove_file(&source_path).await;
        return Err(error);
    }

    let cover_requested = options.embed_thumbnail || options.write_thumbnail;
    let cover = if cover_requested {
        download_cover(track, &destination_dir).await
    } else {
        None
    };

    let result = if process {
        set_status(job_id, JobStatus::Processing);
        run_ffmpeg(
            &source_path,
            &destination,
            cover.as_deref(),
            track,
            format,
            stream.format,
            options,
        )
        .await
    } else {
        if options.overwrite_existing && destination.exists() {
            tokio::fs::remove_file(&destination)
                .await
                .map_err(|error| format!("failed to replace existing file: {error}"))?;
        }
        tokio::fs::rename(&source_path, &destination)
            .await
            .map_err(|error| format!("failed to finish download: {error}"))
    };

    let _ = tokio::fs::remove_file(&source_path).await;
    if let Err(error) = result {
        let _ = tokio::fs::remove_file(&destination).await;
        if let Some(path) = cover {
            let _ = tokio::fs::remove_file(path).await;
        }
        return Err(error);
    }

    if let Some(cover_path) = cover {
        let embed_supported = supports_embedded_cover(format, stream.format);
        if options.write_thumbnail || (options.embed_thumbnail && !embed_supported) {
            let sidecar = destination.with_extension("jpg");
            if options.overwrite_existing && sidecar.exists() {
                let _ = tokio::fs::remove_file(&sidecar).await;
            }
            tokio::fs::rename(&cover_path, &sidecar)
                .await
                .map_err(|error| format!("failed to save cover artwork: {error}"))?;
        } else {
            let _ = tokio::fs::remove_file(cover_path).await;
        }
    }

    Ok(destination)
}

async fn download_stream(
    job_id: &str,
    stream: &server::ytmusic::YtStreamInfo,
    path: &Path,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(20))
        .tcp_nodelay(true)
        .http1_only()
        .build()
        .map_err(|error| format!("failed to build download client: {error}"))?;

    if stream.range_safe
        && let Some(total) = stream.content_length.filter(|total| *total > 0)
    {
        return download_stream_by_ranges(job_id, stream, path, &client, total).await;
    }

    download_stream_sequential(job_id, stream, path, &client).await
}

async fn download_stream_by_ranges(
    job_id: &str,
    stream: &server::ytmusic::YtStreamInfo,
    path: &Path,
    client: &reqwest::Client,
    total: u64,
) -> Result<(), String> {
    const RANGE_SIZE: u64 = 512 * 1024;
    const RANGE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);
    const MAX_ATTEMPTS: u8 = 4;

    let file = tokio::fs::File::create(path)
        .await
        .map_err(|error| format!("failed to create output file: {error}"))?;
    let mut writer = tokio::io::BufWriter::with_capacity(256 * 1024, file);
    let started = Instant::now();
    let mut start = 0u64;

    while start < total {
        let end = (start + RANGE_SIZE - 1).min(total - 1);
        let expected_length = end - start + 1;
        let mut last_error = String::new();
        let mut received = None;

        for attempt in 1..=MAX_ATTEMPTS {
            let request = async {
                let response = client
                    .get(&stream.url)
                    .header(reqwest::header::USER_AGENT, &stream.user_agent)
                    .header(reqwest::header::ACCEPT_ENCODING, "identity")
                    .header(reqwest::header::RANGE, format!("bytes={start}-{end}"))
                    .send()
                    .await
                    .map_err(|error| format!("request failed: {error}"))?;
                if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
                    return Err(format!("expected HTTP 206, received {}", response.status()));
                }
                let bytes = response
                    .bytes()
                    .await
                    .map_err(|error| format!("response failed: {error}"))?;
                if bytes.len() as u64 != expected_length {
                    return Err(format!(
                        "short response: received {} bytes, expected {expected_length}",
                        bytes.len()
                    ));
                }
                Ok::<_, String>(bytes)
            };

            match tokio::time::timeout(RANGE_TIMEOUT, request).await {
                Ok(Ok(bytes)) => {
                    received = Some(bytes);
                    break;
                }
                Ok(Err(error)) => last_error = error,
                Err(_) => last_error = "request timed out".to_string(),
            }

            tracing::warn!(
                target: "youtube_download",
                range_start = start,
                range_end = end,
                attempt,
                error = %last_error,
                "retrying YouTube audio range"
            );
            if attempt < MAX_ATTEMPTS {
                tokio::time::sleep(std::time::Duration::from_millis(250 * u64::from(attempt)))
                    .await;
            }
        }

        let bytes = received.ok_or_else(|| {
            format!("audio range {start}-{end} failed after {MAX_ATTEMPTS} attempts: {last_error}")
        })?;
        writer
            .write_all(&bytes)
            .await
            .map_err(|error| format!("failed to write audio: {error}"))?;
        start = end + 1;
        publish_progress(job_id, start, total, started.elapsed());
    }

    writer
        .flush()
        .await
        .map_err(|error| format!("failed to flush audio: {error}"))?;
    Ok(())
}

async fn download_stream_sequential(
    job_id: &str,
    stream: &server::ytmusic::YtStreamInfo,
    path: &Path,
    client: &reqwest::Client,
) -> Result<(), String> {
    let mut response = client
        .get(&stream.url)
        .header(reqwest::header::USER_AGENT, &stream.user_agent)
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .send()
        .await
        .map_err(|error| format!("download request failed: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("download returned HTTP {}", response.status()));
    }

    let total = stream
        .content_length
        .or_else(|| response.content_length())
        .unwrap_or(0);
    let file = tokio::fs::File::create(path)
        .await
        .map_err(|error| format!("failed to create output file: {error}"))?;
    let mut writer = tokio::io::BufWriter::with_capacity(256 * 1024, file);
    let started = Instant::now();
    let mut downloaded = 0u64;
    let mut last_update = Instant::now();

    loop {
        let chunk = tokio::time::timeout(std::time::Duration::from_secs(120), response.chunk())
            .await
            .map_err(|_| "download timed out while receiving audio".to_string())?
            .map_err(|error| format!("failed while receiving audio: {error}"))?;
        let Some(chunk) = chunk else { break };
        writer
            .write_all(&chunk)
            .await
            .map_err(|error| format!("failed to write audio: {error}"))?;
        downloaded = downloaded.saturating_add(chunk.len() as u64);

        if last_update.elapsed() >= std::time::Duration::from_millis(100)
            || (total > 0 && downloaded >= total)
        {
            publish_progress(job_id, downloaded, total, started.elapsed());
            last_update = Instant::now();
        }
    }
    writer
        .flush()
        .await
        .map_err(|error| format!("failed to flush audio: {error}"))?;
    if total > 0 && downloaded != total {
        return Err(format!(
            "audio response ended early: received {downloaded} bytes, expected {total}"
        ));
    }
    publish_progress(job_id, downloaded, total, started.elapsed());
    Ok(())
}

fn publish_progress(job_id: &str, downloaded: u64, total: u64, elapsed: std::time::Duration) {
    let seconds = elapsed.as_secs_f64().max(0.001);
    let bytes_per_second = downloaded as f64 / seconds;
    let progress = if total > 0 {
        (downloaded as f64 / total as f64 * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };
    let remaining_seconds = if total > downloaded && bytes_per_second > 0.0 {
        ((total - downloaded) as f64 / bytes_per_second).round() as u64
    } else {
        0
    };
    if let Some(job) = JOBS.write().iter_mut().find(|job| job.id == job_id) {
        job.progress = progress;
        job.speed = format_speed(bytes_per_second);
        job.eta = format_eta(remaining_seconds);
        job.status = JobStatus::Downloading;
    }
}

async fn download_cover(track: &Track, directory: &Path) -> Option<PathBuf> {
    let url = match track
        .cover
        .as_deref()
        .map(CoverRef::parse)
        .unwrap_or(CoverRef::None)
    {
        CoverRef::EmbeddedUrl(url) => url,
        _ => return None,
    };
    let response = match reqwest::get(&url).await {
        Ok(response) if response.status().is_success() => response,
        Ok(response) => {
            tracing::warn!(
                target: "youtube_download",
                status = %response.status(),
                "cover download returned an error"
            );
            return None;
        }
        Err(error) => {
            tracing::warn!(target: "youtube_download", %error, "cover download failed");
            return None;
        }
    };
    let bytes = match response.bytes().await {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!(target: "youtube_download", %error, "cover body failed");
            return None;
        }
    };
    let path = directory.join(format!(".{}.cover.jpg", uuid::Uuid::new_v4()));
    match tokio::fs::write(&path, bytes).await {
        Ok(()) => Some(path),
        Err(error) => {
            tracing::warn!(target: "youtube_download", %error, "cover write failed");
            None
        }
    }
}

async fn run_ffmpeg(
    source: &Path,
    destination: &Path,
    cover: Option<&Path>,
    track: &Track,
    format: AudioFormat,
    stream_format: StreamFormat,
    options: &YoutubeDownloadOptions,
) -> Result<(), String> {
    let ffmpeg =
        find_binary("ffmpeg").ok_or_else(|| i18n::t("youtube_download_error_ffmpeg_required"))?;
    let embed_cover = options.embed_thumbnail && supports_embedded_cover(format, stream_format);
    let mut command = tokio::process::Command::new(&ffmpeg);
    command
        .env("PATH", augmented_path())
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-nostdin")
        .arg(if options.overwrite_existing {
            "-y"
        } else {
            "-n"
        })
        .arg("-i")
        .arg(source);

    if embed_cover && let Some(cover) = cover {
        command.arg("-i").arg(cover);
    }

    command.arg("-map").arg("0:a:0");
    if embed_cover && cover.is_some() {
        command
            .arg("-map")
            .arg("1:v:0")
            .arg("-c:v")
            .arg("mjpeg")
            .arg("-disposition:v:0")
            .arg("attached_pic")
            .arg("-metadata:s:v")
            .arg("title=Album cover")
            .arg("-metadata:s:v")
            .arg("comment=Cover (front)");
    } else {
        command.arg("-vn");
    }

    match format {
        AudioFormat::Original => {
            command.arg("-c:a").arg("copy");
        }
        AudioFormat::Mp3 => {
            command
                .arg("-c:a")
                .arg("libmp3lame")
                .arg("-q:a")
                .arg("0")
                .arg("-id3v2_version")
                .arg("3");
        }
        AudioFormat::Flac => {
            command.arg("-c:a").arg("flac");
        }
        AudioFormat::Opus => {
            command.arg("-c:a").arg("libopus").arg("-b:a").arg("192k");
        }
        AudioFormat::Wav => {
            command.arg("-c:a").arg("pcm_s16le");
        }
    }

    if options.embed_metadata {
        command
            .arg("-metadata")
            .arg(format!("title={}", track.title))
            .arg("-metadata")
            .arg(format!("artist={}", track.artist));
        if !track.album.trim().is_empty() {
            command
                .arg("-metadata")
                .arg(format!("album={}", track.album));
        }
        if let Some(number) = track.track_number {
            command.arg("-metadata").arg(format!("track={number}"));
        }
        command.arg("-metadata").arg(format!(
            "purl=https://music.youtube.com/watch?v={}",
            track.id.key()
        ));
    }

    command.arg(destination);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.as_std_mut().creation_flags(0x0800_0000);
    }

    let output = command
        .output()
        .await
        .map_err(|error| format!("failed to start ffmpeg: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if message.is_empty() {
            format!("ffmpeg exited with {}", output.status)
        } else {
            format!("ffmpeg: {message}")
        })
    }
}

fn requires_ffmpeg(format: AudioFormat, options: &YoutubeDownloadOptions) -> bool {
    format.needs_conversion() || options.embed_metadata || options.embed_thumbnail
}

fn supports_embedded_cover(format: AudioFormat, stream_format: StreamFormat) -> bool {
    matches!(format, AudioFormat::Mp3 | AudioFormat::Flac)
        || (matches!(format, AudioFormat::Original) && matches!(stream_format, StreamFormat::M4a))
}

fn output_extension(
    format: AudioFormat,
    stream_format: StreamFormat,
    processed: bool,
) -> &'static str {
    match format {
        AudioFormat::Original if processed && matches!(stream_format, StreamFormat::Webm) => "mka",
        AudioFormat::Original => stream_format.extension(),
        AudioFormat::Mp3 => "mp3",
        AudioFormat::Flac => "flac",
        AudioFormat::Opus => "opus",
        AudioFormat::Wav => "wav",
    }
}

fn output_root(configured: &str) -> PathBuf {
    let configured = configured.trim();
    if !configured.is_empty() {
        return PathBuf::from(configured);
    }
    directories::UserDirs::new()
        .and_then(|dirs| dirs.audio_dir().map(Path::to_path_buf))
        .or_else(|| directories::UserDirs::new().map(|dirs| dirs.home_dir().to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn output_stem(track: &Track) -> String {
    let title = sanitize_component(&track.title);
    if track.artist.trim().is_empty() {
        title
    } else {
        let artist = sanitize_component(&track.artist);
        format!("{artist} - {title}")
    }
}

fn sanitize_component(value: &str) -> String {
    let sanitized: String = value
        .trim()
        .chars()
        .map(|character| match character {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
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

fn destination_path(wanted: &Path, overwrite: bool) -> PathBuf {
    if overwrite || !wanted.exists() {
        return wanted.to_path_buf();
    }
    let parent = wanted.parent().unwrap_or_else(|| Path::new("."));
    let stem = wanted
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("download");
    let extension = wanted.extension().and_then(|value| value.to_str());
    for index in 2..10_000 {
        let name = match extension {
            Some(extension) => format!("{stem} ({index}).{extension}"),
            None => format!("{stem} ({index})"),
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    parent.join(format!("{stem} - {}", uuid::Uuid::new_v4()))
}

fn set_status(job_id: &str, status: JobStatus) {
    if let Some(job) = JOBS.write().iter_mut().find(|job| job.id == job_id) {
        job.status = status;
    }
}

fn format_speed(bytes_per_second: f64) -> String {
    if bytes_per_second >= 1024.0 * 1024.0 {
        format!("{:.1} MiB/s", bytes_per_second / (1024.0 * 1024.0))
    } else {
        format!("{:.0} KiB/s", bytes_per_second / 1024.0)
    }
}

fn format_eta(seconds: u64) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn validate_output_directory(path: &Path) -> Result<(), String> {
    if let Some(volume) = unavailable_macos_volume(path) {
        return Err(i18n::t_with(
            "youtube_download_error_output_prepare",
            &[(
                "error",
                format!("volume is not mounted: {}", volume.display()),
            )],
        ));
    }
    if path.exists() && !path.is_dir() {
        return Err(i18n::t_with(
            "youtube_download_error_output_not_directory",
            &[("path", path.display().to_string())],
        ));
    }
    fs::create_dir_all(path).map_err(|error| {
        i18n::t_with(
            "youtube_download_error_output_prepare",
            &[("error", error.to_string())],
        )
    })?;

    let probe = path.join(format!(".kopuz-write-test-{}", uuid::Uuid::new_v4()));
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .map_err(|_| {
            i18n::t_with(
                "youtube_download_error_output_not_writable",
                &[("path", path.display().to_string())],
            )
        })?;
    let _ = fs::remove_file(probe);
    Ok(())
}

#[cfg(target_os = "macos")]
fn unavailable_macos_volume(path: &Path) -> Option<PathBuf> {
    use std::path::Component;

    let mut components = path.components();
    if components.next() != Some(Component::RootDir)
        || components.next() != Some(Component::Normal("Volumes".as_ref()))
    {
        return None;
    }
    let Component::Normal(volume_name) = components.next()? else {
        return None;
    };
    let volume = Path::new("/Volumes").join(volume_name);
    (!volume.exists()).then_some(volume)
}

#[cfg(not(target_os = "macos"))]
fn unavailable_macos_volume(_path: &Path) -> Option<PathBuf> {
    None
}

fn search_dirs() -> &'static [PathBuf] {
    static DIRECTORIES: std::sync::OnceLock<Vec<PathBuf>> = std::sync::OnceLock::new();
    DIRECTORIES.get_or_init(|| {
        let mut directories: Vec<PathBuf> =
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect();
        if let Some(shell) = std::env::var_os("SHELL")
            && let Ok(output) = std::process::Command::new(shell)
                .arg("-lc")
                .arg("printf %s \"$PATH\"")
                .output()
            && output.status.success()
        {
            let path = String::from_utf8_lossy(&output.stdout);
            for directory in std::env::split_paths(path.trim()) {
                if !directories.contains(&directory) {
                    directories.push(directory);
                }
            }
        }
        directories
    })
}

fn augmented_path() -> std::ffi::OsString {
    std::env::join_paths(search_dirs()).unwrap_or_default()
}

fn find_binary(name: &str) -> Option<String> {
    let executable = if cfg!(target_os = "windows") && !name.ends_with(".exe") {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    search_dirs().iter().find_map(|directory| {
        let candidate = directory.join(&executable);
        candidate
            .is_file()
            .then(|| candidate.to_string_lossy().into_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_cross_platform_filename_characters() {
        assert_eq!(sanitize_component("  A/B: C?  "), "A_B_ C_");
        assert_eq!(sanitize_component("..."), "Untitled");
    }

    #[test]
    fn original_output_keeps_native_container_without_processing() {
        assert_eq!(
            output_extension(AudioFormat::Original, StreamFormat::Webm, false),
            "webm"
        );
        assert_eq!(
            output_extension(AudioFormat::Original, StreamFormat::M4a, false),
            "m4a"
        );
    }

    #[test]
    fn processed_webm_uses_audio_matroska_container() {
        assert_eq!(
            output_extension(AudioFormat::Original, StreamFormat::Webm, true),
            "mka"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn reports_an_unmounted_external_volume() {
        let missing = format!("kopuz-missing-{}", uuid::Uuid::new_v4());
        let path = Path::new("/Volumes").join(&missing).join("Music");

        assert_eq!(
            unavailable_macos_volume(&path),
            Some(Path::new("/Volumes").join(missing))
        );
    }
}
