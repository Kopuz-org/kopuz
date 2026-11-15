//! First-run content for the in-process library: six short sine tones,
//! written as plain 16-bit WAV files into `<filesDir>/spike-music/`, added as
//! a folder source and scanned, all through `KopuzApi` (the same calls as
//! `kopuzctl source add folders --activate` and `kopuzctl jobs start scan`).
//! Plain WAV carries no tags, so the file names become the titles.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use api::{
    ApiError, FieldValue, JobKind, KopuzApi, Page, SourceDraft, TrackFilter, encode_directories,
};

const SOURCE_NAME: &str = "Spike test music";
/// `daemon::services::FOLDERS` and `DIRECTORIES`: the folders the daemon scans itself.
const FOLDERS_SERVICE: &str = "folders";
const DIRECTORIES_FIELD: &str = "directories";

const SAMPLE_RATE: u32 = 44_100;
/// (note, frequency in Hz); tone i lasts 10 + 2 i seconds.
const TONES: [(&str, f32); 6] = [
    ("A4", 440.00),
    ("B4", 493.88),
    ("C5", 523.25),
    ("D5", 587.33),
    ("E5", 659.25),
    ("G5", 783.99),
];
const SCAN_TIMEOUT: Duration = Duration::from_secs(90);
const RESCAN_EVERY: Duration = Duration::from_secs(20);

fn music_dir() -> PathBuf {
    crate::inproc::files_dir().join("spike-music")
}

fn internal(context: &str, error: impl std::fmt::Display) -> ApiError {
    ApiError::internal(format!("{context}: {error}"))
}

/// Mono 16-bit PCM WAV: `secs` seconds of `freq` Hz at a quiet level, with a
/// 50 ms fade at each end so playback starts and stops without a click.
fn sine_wav(freq: f32, secs: u32) -> Vec<u8> {
    let samples = SAMPLE_RATE * secs;
    let data_len = samples * 2;
    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes()); // byte rate
    wav.extend_from_slice(&2u16.to_le_bytes()); // block align
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    let fade = SAMPLE_RATE / 20;
    let step = std::f32::consts::TAU * freq / SAMPLE_RATE as f32;
    for n in 0..samples {
        let edge = n.min(samples - 1 - n);
        let gain = if edge < fade {
            edge as f32 / fade as f32
        } else {
            1.0
        };
        let phase = (n as f32 * step) % std::f32::consts::TAU;
        let value = (phase.sin() * gain * 0.25 * i16::MAX as f32) as i16;
        wav.extend_from_slice(&value.to_le_bytes());
    }
    wav
}

/// Write the tones that are missing; a file appears under its final name
/// only when complete, so a scan never reads half a file.
fn write_tones(dir: &Path) -> std::io::Result<usize> {
    std::fs::create_dir_all(dir)?;
    for (i, (note, freq)) in TONES.iter().enumerate() {
        let secs = 10 + 2 * i as u32;
        let path = dir.join(format!("Spike tone {} – {note}.wav", i + 1));
        let wav = sine_wav(*freq, secs);
        if std::fs::metadata(&path).is_ok_and(|meta| meta.len() == wav.len() as u64) {
            continue;
        }
        let partial = dir.join(format!(".tone-{}.partial", i + 1));
        std::fs::write(&partial, &wav)?;
        std::fs::rename(&partial, &path)?;
    }
    Ok(TONES.len())
}

async fn track_total(api: &dyn KopuzApi) -> Result<u32, ApiError> {
    let page = Page {
        offset: 0,
        limit: 1,
    };
    Ok(api.tracks(TrackFilter::default(), page).await?.total)
}

/// Write the tones, make their folder the active source, scan it, and wait
/// until the tracks are listed. Returns how many tracks the library has.
pub async fn seed(api: &dyn KopuzApi) -> Result<u32, ApiError> {
    let dir = music_dir();
    let written = {
        let dir = dir.clone();
        tokio::task::spawn_blocking(move || write_tones(&dir))
            .await
            .map_err(|error| internal("writing the test tones", error))?
            .map_err(|error| internal("writing the test tones", error))?
    };
    log::info!("SPIKE seeding files={written} dir={}", dir.display());

    // Reuse the source a cut-short earlier run added rather than adding a second.
    let existing = api
        .sources()
        .await?
        .into_iter()
        .find(|source| source.name == SOURCE_NAME && source.service.id == FOLDERS_SERVICE)
        .map(|source| source.id);
    let directories = encode_directories(&[dir.to_string_lossy().into_owned()]);
    let source = api
        .upsert_source(SourceDraft {
            id: existing,
            name: SOURCE_NAME.into(),
            service: FOLDERS_SERVICE.into(),
            values: vec![FieldValue::new(DIRECTORIES_FIELD, directories)],
            secrets: Vec::new(),
        })
        .await?;
    let source = if source.active {
        source
    } else {
        api.switch_source(source.id).await?
    };
    log::info!("SPIKE seeding source={} active={}", source.id, source.active);

    // Switching sources may already have started a scan; a second request
    // is then refused, which is fine.
    let start_scan = || async move {
        if let Err(error) = api.start_job(JobKind::Scan).await {
            log::info!("SPIKE seeding scan request: {}", error.message);
        }
    };
    start_scan().await;
    let started = Instant::now();
    let mut last_scan = started;
    let (mut last, mut steady) = (0, 0);
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let total = track_total(api).await?;
        steady = if total == last { steady + 1 } else { 0 };
        last = total;
        // Done once every tone is listed, or once the count has not grown for 3 s.
        if total as usize >= TONES.len() || (total > 0 && steady >= 3) {
            return Ok(total);
        }
        if started.elapsed() >= SCAN_TIMEOUT {
            return Ok(total);
        }
        if last_scan.elapsed() >= RESCAN_EVERY {
            start_scan().await;
            last_scan = Instant::now();
        }
    }
}
