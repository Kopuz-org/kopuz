use std::sync::Arc;

use config::YtdlpOptions;
use dioxus::core::spawn_forever;
use dioxus::prelude::*;

pub(crate) static JOBS: GlobalSignal<Vec<DownloadJob>> = Signal::global(Vec::new);

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
    Opus,
    Wav,
    Video,
}

impl AudioFormat {
    fn label_key(self) -> &'static str {
        match self {
            Self::BestAudio => "ytdlp_format_best_audio",
            Self::Mp3 => "ytdlp_format_mp3",
            Self::Flac => "ytdlp_format_flac",
            Self::Opus => "ytdlp_format_opus",
            Self::Wav => "ytdlp_format_wav",
            Self::Video => "ytdlp_format_video",
        }
    }

    pub fn label(self) -> String {
        i18n::t(self.label_key())
    }

    fn from_str(value: &str) -> Self {
        match value {
            "MP3" => Self::Mp3,
            "FLAC" => Self::Flac,
            "OPUS" => Self::Opus,
            "WAV" => Self::Wav,
            "Video (MP4)" => Self::Video,
            _ => Self::BestAudio,
        }
    }

    fn api(self) -> api::YtdlpAudioFormat {
        match self {
            Self::BestAudio => api::YtdlpAudioFormat::Best,
            Self::Mp3 => api::YtdlpAudioFormat::Mp3,
            Self::Flac => api::YtdlpAudioFormat::Flac,
            Self::Opus => api::YtdlpAudioFormat::Opus,
            Self::Wav => api::YtdlpAudioFormat::Wav,
            Self::Video => api::YtdlpAudioFormat::Video,
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

pub fn run_preflight_checks(url: &str, _out_dir: &str) -> Result<(), String> {
    if JOBS.read().iter().any(|job| {
        job.url.trim() == url
            && matches!(
                job.status,
                JobStatus::Pending | JobStatus::Downloading | JobStatus::Processing
            )
    }) {
        Err(i18n::t("ytdlp_error_duplicate_active"))
    } else {
        Ok(())
    }
}

fn apply_status(job: &mut DownloadJob, status: &api::JobStatus) {
    job.url = status.request.clone().unwrap_or_else(|| job.url.clone());
    job.title = status.title.clone().unwrap_or_else(|| job.title.clone());
    job.format = status
        .format
        .as_deref()
        .map(AudioFormat::from_str)
        .unwrap_or(job.format);
    job.progress = match (status.current, status.total) {
        (Some(current), Some(total)) if total > 0 => current as f64 / total as f64 * 100.0,
        _ => job.progress,
    };
    job.speed = status.speed.clone().unwrap_or_default();
    job.eta = status.eta.clone().unwrap_or_default();
    job.status = match status.state {
        api::JobState::Running if status.phase == "processing" => JobStatus::Processing,
        api::JobState::Running => JobStatus::Downloading,
        api::JobState::Finished => {
            job.progress = 100.0;
            JobStatus::Completed
        }
        api::JobState::Failed => JobStatus::Failed(
            status
                .error
                .as_ref()
                .map(|error| error.message.clone())
                .unwrap_or_else(|| "yt-dlp failed".to_string()),
        ),
        api::JobState::Cancelled => JobStatus::Failed("yt-dlp was cancelled".to_string()),
        api::JobState::Unknown => job.status.clone(),
    };
}

pub fn start_download(url: String, out: String, format: AudioFormat, options: YtdlpOptions) {
    let api = consume_context::<Arc<dyn api::KopuzApi>>();
    let temporary_id = uuid::Uuid::new_v4().to_string();
    JOBS.write().insert(
        0,
        DownloadJob {
            id: temporary_id.clone(),
            url: url.clone(),
            title: url.clone(),
            format,
            progress: 0.0,
            status: JobStatus::Pending,
            speed: String::new(),
            eta: String::new(),
        },
    );
    spawn_forever(async move {
        let request = api::YtdlpRequest {
            url,
            output_dir: out,
            format: format.api(),
            options: serde_json::to_value(options).unwrap_or(serde_json::Value::Null),
        };
        let job_ref = match api.start_ytdlp(request).await {
            Ok(job_ref) => job_ref,
            Err(error) => {
                if let Some(job) = JOBS.write().iter_mut().find(|job| job.id == temporary_id) {
                    job.status = JobStatus::Failed(error.message);
                }
                return;
            }
        };
        if let Some(job) = JOBS.write().iter_mut().find(|job| job.id == temporary_id) {
            job.id.clone_from(&job_ref.job_id);
            job.status = JobStatus::Downloading;
        }
        loop {
            match api.jobs().await {
                Ok(statuses) => {
                    let Some(status) = statuses.iter().find(|status| status.id == job_ref.job_id)
                    else {
                        break;
                    };
                    let done = !matches!(status.state, api::JobState::Running);
                    if let Some(job) = JOBS.write().iter_mut().find(|job| job.id == job_ref.job_id)
                    {
                        apply_status(job, status);
                    }
                    if done {
                        break;
                    }
                }
                Err(error) => tracing::warn!(%error, "could not refresh yt-dlp job"),
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    });
}
