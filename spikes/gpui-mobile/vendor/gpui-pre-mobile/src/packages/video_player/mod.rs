//! Video playback for Android and iOS.
//!
//! Provides a cross-platform video player API backed by:
//! - Android: MediaPlayer via JNI
//! - iOS: AVPlayer via Objective-C
//!
//! Uses the platform view system for native surface embedding.
//!
//! Feature-gated behind `video_player`.

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "ios")]
mod ios;

use crate::platform_view::{
    PlatformViewBounds, PlatformViewHandle, PlatformViewParams, PlatformViewRegistry,
};
use std::sync::Arc;

/// Video player state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VideoPlayerState {
    Uninitialized,
    Initialized,
    Playing,
    Paused,
    Completed,
    Error,
}

/// Video information returned after setting a source.
#[derive(Debug, Clone, Copy)]
pub struct VideoInfo {
    pub duration_ms: u64,
    pub width: u32,
    pub height: u32,
}

/// A video player instance.
///
/// Each `VideoPlayer` owns a platform-specific media player identified by an
/// integer ID. Resources are released automatically on [`Drop`].
///
/// The video surface is embedded via the platform view system. Call
/// [`show_surface`] to create the native view, or use [`platform_view_handle`]
/// to embed it in a GPUI element via `platform_view_element()`.
pub struct VideoPlayer {
    id: u32,
    /// Platform view handle for the video surface (set when surface is showing).
    surface_handle: Option<Arc<PlatformViewHandle>>,
}

impl std::fmt::Debug for VideoPlayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoPlayer")
            .field("id", &self.id)
            .field("has_surface", &self.surface_handle.is_some())
            .finish()
    }
}

/// Register the "video_player" platform view factory.
///
/// Called lazily on first use. Subsequent calls are no-ops.
fn ensure_factory_registered() {
    let registry = PlatformViewRegistry::global();
    if !registry.has_factory("video_player") {
        #[cfg(target_os = "android")]
        {
            use crate::android::platform_view::AndroidPlatformViewFactory;
            registry.register(
                "video_player",
                Box::new(AndroidPlatformViewFactory::new("video_player")),
            );
        }
        #[cfg(target_os = "ios")]
        {
            use crate::ios::platform_view::IosPlatformViewFactory;
            registry.register(
                "video_player",
                Box::new(IosPlatformViewFactory::new("video_player")),
            );
        }
    }
}

/// Get the raw AVPlayer pointer for a given player ID (iOS only).
///
/// Used by `IosPlatformView` to create an `AVPlayerLayer` during
/// platform view construction.
#[cfg(target_os = "ios")]
pub fn ios_get_player(id: u32) -> Option<*mut objc2::runtime::AnyObject> {
    ios::get_player_ptr(id)
}

impl VideoPlayer {
    /// Create a new video player.
    pub fn new() -> Result<Self, String> {
        #[cfg(target_os = "ios")]
        let id = ios::create_player()?;
        #[cfg(target_os = "android")]
        let id = android::create_player()?;
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        return Err("VideoPlayer is not supported on this platform".into());

        #[cfg(any(target_os = "ios", target_os = "android"))]
        Ok(VideoPlayer {
            id,
            surface_handle: None,
        })
    }

    /// Set video source from a URL.
    pub fn set_url(&self, url: &str) -> Result<VideoInfo, String> {
        #[cfg(target_os = "ios")]
        {
            ios::set_url(self.id, url)
        }
        #[cfg(target_os = "android")]
        {
            android::set_url(self.id, url)
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            let _ = url;
            Err("not supported".into())
        }
    }

    /// Set video source from a file path.
    pub fn set_file_path(&self, path: &str) -> Result<VideoInfo, String> {
        #[cfg(target_os = "ios")]
        {
            ios::set_file_path(self.id, path)
        }
        #[cfg(target_os = "android")]
        {
            android::set_file_path(self.id, path)
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            let _ = path;
            Err("not supported".into())
        }
    }

    /// Start or resume playback.
    pub fn play(&self) -> Result<(), String> {
        #[cfg(target_os = "ios")]
        {
            ios::play(self.id)
        }
        #[cfg(target_os = "android")]
        {
            android::play(self.id)
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            Err("not supported".into())
        }
    }

    /// Pause playback.
    pub fn pause(&self) -> Result<(), String> {
        #[cfg(target_os = "ios")]
        {
            ios::pause(self.id)
        }
        #[cfg(target_os = "android")]
        {
            android::pause(self.id)
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            Err("not supported".into())
        }
    }

    /// Seek to a position in milliseconds.
    pub fn seek(&self, position_ms: u64) -> Result<(), String> {
        #[cfg(target_os = "ios")]
        {
            ios::seek(self.id, position_ms)
        }
        #[cfg(target_os = "android")]
        {
            android::seek(self.id, position_ms)
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            let _ = position_ms;
            Err("not supported".into())
        }
    }

    /// Set volume (0.0 to 1.0).
    pub fn set_volume(&self, volume: f32) -> Result<(), String> {
        #[cfg(target_os = "ios")]
        {
            ios::set_volume(self.id, volume)
        }
        #[cfg(target_os = "android")]
        {
            android::set_volume(self.id, volume)
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            let _ = volume;
            Err("not supported".into())
        }
    }

    /// Set playback speed (e.g. 1.0 for normal, 2.0 for double speed).
    pub fn set_speed(&self, speed: f32) -> Result<(), String> {
        #[cfg(target_os = "ios")]
        {
            ios::set_speed(self.id, speed)
        }
        #[cfg(target_os = "android")]
        {
            android::set_speed(self.id, speed)
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            let _ = speed;
            Err("not supported".into())
        }
    }

    /// Enable or disable looping.
    pub fn set_looping(&self, looping: bool) -> Result<(), String> {
        #[cfg(target_os = "ios")]
        {
            ios::set_looping(self.id, looping)
        }
        #[cfg(target_os = "android")]
        {
            android::set_looping(self.id, looping)
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            let _ = looping;
            Err("not supported".into())
        }
    }

    /// Get current playback position in milliseconds.
    pub fn position(&self) -> Result<u64, String> {
        #[cfg(target_os = "ios")]
        {
            ios::position(self.id)
        }
        #[cfg(target_os = "android")]
        {
            android::position(self.id)
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            Err("not supported".into())
        }
    }

    /// Get total duration in milliseconds.
    pub fn duration(&self) -> Result<u64, String> {
        #[cfg(target_os = "ios")]
        {
            ios::duration(self.id)
        }
        #[cfg(target_os = "android")]
        {
            android::duration(self.id)
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            Err("not supported".into())
        }
    }

    /// Get video dimensions as `(width, height)`.
    pub fn video_size(&self) -> Result<(u32, u32), String> {
        #[cfg(target_os = "ios")]
        {
            ios::video_size(self.id)
        }
        #[cfg(target_os = "android")]
        {
            android::video_size(self.id)
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            Err("not supported".into())
        }
    }

    /// Check if currently playing.
    pub fn is_playing(&self) -> Result<bool, String> {
        #[cfg(target_os = "ios")]
        {
            ios::is_playing(self.id)
        }
        #[cfg(target_os = "android")]
        {
            android::is_playing(self.id)
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            Err("not supported".into())
        }
    }

    /// Show the native video surface at the given position and size (in logical pixels).
    ///
    /// On the first call, creates a platform view of type "video_player" via the
    /// platform view registry. On subsequent calls, updates the bounds of the
    /// existing view without recreating it.
    ///
    /// Use [`platform_view_handle`] to get an `Arc<PlatformViewHandle>` for
    /// embedding in a GPUI element via `platform_view_element()`.
    pub fn show_surface(&mut self, x: f32, y: f32, width: f32, height: f32) -> Result<(), String> {
        let bounds = PlatformViewBounds {
            x,
            y,
            width,
            height,
        };

        // If surface already exists, just update bounds
        if let Some(ref handle) = self.surface_handle {
            handle.set_bounds(bounds);
            handle.set_visible(true);
            return Ok(());
        }

        // First call — create the platform view
        ensure_factory_registered();

        let mut creation_params = std::collections::HashMap::new();
        creation_params.insert("player_id".to_string(), self.id.to_string());

        let params = PlatformViewParams {
            bounds,
            creation_params,
        };

        let handle = PlatformViewRegistry::global().create_view("video_player", params)?;
        self.surface_handle = Some(Arc::new(handle));
        Ok(())
    }

    /// Hide (remove) the native video surface.
    pub fn hide_surface(&mut self) -> Result<(), String> {
        if let Some(handle) = self.surface_handle.take() {
            handle.dispose();
        }
        Ok(())
    }

    /// Get the platform view handle for the video surface.
    ///
    /// Returns `None` if `show_surface()` has not been called.
    /// Pass this to `platform_view_element()` to embed the video
    /// in the GPUI render tree.
    pub fn platform_view_handle(&self) -> Option<Arc<PlatformViewHandle>> {
        self.surface_handle.clone()
    }

    /// Release player resources.
    ///
    /// Called automatically on [`Drop`], but can be invoked early to free
    /// resources sooner.
    pub fn dispose(&mut self) -> Result<(), String> {
        // Dispose platform view first
        self.hide_surface()?;

        #[cfg(target_os = "ios")]
        {
            ios::dispose(self.id)
        }
        #[cfg(target_os = "android")]
        {
            android::dispose(self.id)
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            Ok(())
        }
    }
}

impl Drop for VideoPlayer {
    fn drop(&mut self) {
        let _ = self.dispose();
    }
}
