//! Microphone — audio recording support.

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "ios")]
mod ios;

/// Audio recording format.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AudioFormat {
    /// AAC encoding (.m4a)
    #[default]
    Aac,
    /// WAV encoding (.wav)
    Wav,
    /// AMR encoding (.amr) — Android only
    Amr,
}

/// Recording configuration.
#[derive(Debug, Clone)]
pub struct RecordingConfig {
    /// Output format.
    pub format: AudioFormat,
    /// Sample rate in Hz (default: 44100).
    pub sample_rate: u32,
    /// Number of channels (default: 1 for mono).
    pub channels: u8,
    /// Bit rate in bits per second (default: 128000).
    pub bit_rate: u32,
}

impl Default for RecordingConfig {
    fn default() -> Self {
        Self {
            format: AudioFormat::Aac,
            sample_rate: 44100,
            channels: 1,
            bit_rate: 128000,
        }
    }
}

/// Result of a completed recording.
#[derive(Debug, Clone)]
pub struct Recording {
    /// Path to the recorded audio file.
    pub path: String,
    /// Duration in milliseconds.
    pub duration_ms: u64,
}

/// Check if audio recording is available.
pub fn is_available() -> bool {
    #[cfg(target_os = "ios")]
    {
        ios::is_available()
    }
    #[cfg(target_os = "android")]
    {
        android::is_available()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        false
    }
}

/// Start recording audio with the given configuration.
/// Returns the path where audio will be saved.
pub fn start_recording(config: &RecordingConfig) -> Result<String, String> {
    #[cfg(target_os = "ios")]
    {
        ios::start_recording(config)
    }
    #[cfg(target_os = "android")]
    {
        android::start_recording(config)
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        let _ = config;
        Err("microphone is only available on iOS and Android".into())
    }
}

/// Stop recording and return the recording result.
pub fn stop_recording() -> Result<Recording, String> {
    #[cfg(target_os = "ios")]
    {
        ios::stop_recording()
    }
    #[cfg(target_os = "android")]
    {
        android::stop_recording()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        Err("microphone is only available on iOS and Android".into())
    }
}

/// Check if currently recording.
pub fn is_recording() -> bool {
    #[cfg(target_os = "ios")]
    {
        ios::is_recording()
    }
    #[cfg(target_os = "android")]
    {
        android::is_recording()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        false
    }
}

/// Pause the current recording (if supported).
pub fn pause_recording() -> Result<(), String> {
    #[cfg(target_os = "ios")]
    {
        ios::pause_recording()
    }
    #[cfg(target_os = "android")]
    {
        android::pause_recording()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        Err("microphone is only available on iOS and Android".into())
    }
}

/// Resume a paused recording.
pub fn resume_recording() -> Result<(), String> {
    #[cfg(target_os = "ios")]
    {
        ios::resume_recording()
    }
    #[cfg(target_os = "android")]
    {
        android::resume_recording()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        Err("microphone is only available on iOS and Android".into())
    }
}

/// Get the current amplitude (0.0 to 1.0) if recording.
pub fn get_amplitude() -> Result<f64, String> {
    #[cfg(target_os = "ios")]
    {
        ios::get_amplitude()
    }
    #[cfg(target_os = "android")]
    {
        android::get_amplitude()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        Err("microphone is only available on iOS and Android".into())
    }
}
