//! Audio playback for Android and iOS.
//!
//! Provides a cross-platform audio player API backed by:
//! - Android: MediaPlayer via JNI
//! - iOS: AVAudioPlayer via Objective-C
//!
//! Feature-gated behind `audio`.

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "ios")]
mod ios;

/// Audio player state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlayerState {
    Idle,
    Loading,
    Ready,
    Playing,
    Paused,
    Completed,
}

/// Loop mode.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum LoopMode {
    #[default]
    Off,
    One,
    All,
}

/// Audio player instance (wraps a platform-specific player ID).
#[derive(Debug)]
#[allow(dead_code)]
pub struct AudioPlayer {
    id: u32,
}

impl AudioPlayer {
    /// Create a new audio player.
    pub fn new() -> Result<Self, String> {
        #[cfg(target_os = "android")]
        {
            android::create().map(|id| Self { id })
        }
        #[cfg(target_os = "ios")]
        {
            ios::create().map(|id| Self { id })
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            Err("Audio not supported on this platform".into())
        }
    }

    /// Set the audio source from a URL (http/https/file).
    /// Returns the duration in milliseconds, or None if unknown.
    pub fn set_url(&self, url: &str) -> Result<Option<u64>, String> {
        #[cfg(target_os = "android")]
        {
            android::set_url(self.id, url)
        }
        #[cfg(target_os = "ios")]
        {
            ios::set_url(self.id, url)
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            let _ = url;
            Err("Audio not supported on this platform".into())
        }
    }

    /// Set the audio source from a file path.
    pub fn set_file_path(&self, path: &str) -> Result<Option<u64>, String> {
        #[cfg(target_os = "android")]
        {
            android::set_url(self.id, path)
        }
        #[cfg(target_os = "ios")]
        {
            ios::set_file_path(self.id, path)
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            let _ = path;
            Err("Audio not supported on this platform".into())
        }
    }

    /// Start or resume playback.
    pub fn play(&self) -> Result<(), String> {
        #[cfg(target_os = "android")]
        {
            android::play(self.id)
        }
        #[cfg(target_os = "ios")]
        {
            ios::play(self.id)
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            Err("Audio not supported on this platform".into())
        }
    }

    /// Pause playback.
    pub fn pause(&self) -> Result<(), String> {
        #[cfg(target_os = "android")]
        {
            android::pause(self.id)
        }
        #[cfg(target_os = "ios")]
        {
            ios::pause(self.id)
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            Err("Audio not supported on this platform".into())
        }
    }

    /// Stop playback and release resources.
    pub fn stop(&self) -> Result<(), String> {
        #[cfg(target_os = "android")]
        {
            android::stop(self.id)
        }
        #[cfg(target_os = "ios")]
        {
            ios::stop(self.id)
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            Err("Audio not supported on this platform".into())
        }
    }

    /// Seek to position in milliseconds.
    pub fn seek(&self, position_ms: u64) -> Result<(), String> {
        #[cfg(target_os = "android")]
        {
            android::seek(self.id, position_ms)
        }
        #[cfg(target_os = "ios")]
        {
            ios::seek(self.id, position_ms)
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            let _ = position_ms;
            Err("Audio not supported on this platform".into())
        }
    }

    /// Set volume (0.0 to 1.0).
    pub fn set_volume(&self, volume: f32) -> Result<(), String> {
        #[cfg(target_os = "android")]
        {
            android::set_volume(self.id, volume)
        }
        #[cfg(target_os = "ios")]
        {
            ios::set_volume(self.id, volume)
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            let _ = volume;
            Err("Audio not supported on this platform".into())
        }
    }

    /// Set playback speed (1.0 = normal).
    pub fn set_speed(&self, speed: f32) -> Result<(), String> {
        #[cfg(target_os = "android")]
        {
            android::set_speed(self.id, speed)
        }
        #[cfg(target_os = "ios")]
        {
            ios::set_speed(self.id, speed)
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            let _ = speed;
            Err("Audio not supported on this platform".into())
        }
    }

    /// Set loop mode.
    pub fn set_loop_mode(&self, mode: LoopMode) -> Result<(), String> {
        #[cfg(target_os = "android")]
        {
            android::set_loop_mode(self.id, mode)
        }
        #[cfg(target_os = "ios")]
        {
            ios::set_loop_mode(self.id, mode)
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            let _ = mode;
            Err("Audio not supported on this platform".into())
        }
    }

    /// Get current playback position in milliseconds.
    pub fn position(&self) -> Result<u64, String> {
        #[cfg(target_os = "android")]
        {
            android::get_position(self.id)
        }
        #[cfg(target_os = "ios")]
        {
            ios::get_position(self.id)
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            Err("Audio not supported on this platform".into())
        }
    }

    /// Get total duration in milliseconds (0 if unknown).
    pub fn duration(&self) -> Result<u64, String> {
        #[cfg(target_os = "android")]
        {
            android::get_duration(self.id)
        }
        #[cfg(target_os = "ios")]
        {
            ios::get_duration(self.id)
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            Err("Audio not supported on this platform".into())
        }
    }

    /// Get current player state.
    pub fn state(&self) -> Result<PlayerState, String> {
        #[cfg(target_os = "android")]
        {
            android::get_state(self.id)
        }
        #[cfg(target_os = "ios")]
        {
            ios::get_state(self.id)
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            Err("Audio not supported on this platform".into())
        }
    }

    /// Check if currently playing.
    pub fn is_playing(&self) -> Result<bool, String> {
        #[cfg(target_os = "android")]
        {
            android::is_playing(self.id)
        }
        #[cfg(target_os = "ios")]
        {
            ios::is_playing(self.id)
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            Err("Audio not supported on this platform".into())
        }
    }

    fn dispose(&self) -> Result<(), String> {
        #[cfg(target_os = "android")]
        {
            android::dispose(self.id)
        }
        #[cfg(target_os = "ios")]
        {
            ios::dispose(self.id)
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            Ok(())
        }
    }
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        let _ = self.dispose();
    }
}
