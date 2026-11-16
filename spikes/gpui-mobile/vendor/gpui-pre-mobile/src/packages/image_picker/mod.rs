//! Image and video picker for selecting media from gallery or camera.
//!
//! Provides a cross-platform media picker API backed by:
//! - iOS: `PHPickerViewController` / `UIImagePickerController` via Objective-C
//! - Android: `Intent.ACTION_PICK` / `MediaStore.ACTION_IMAGE_CAPTURE` via JNI
//!
//! Inspired by [image_picker](https://pub.dev/packages/image_picker).
//!
//! Feature-gated behind `image_picker`.

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "ios")]
mod ios;

/// Source for picking an image or video.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageSource {
    /// Pick from the device photo library / gallery.
    Gallery,
    /// Capture using the device camera.
    Camera,
}

/// Preferred camera device when using `ImageSource::Camera`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CameraDevice {
    /// Use the rear-facing camera.
    #[default]
    Rear,
    /// Use the front-facing (selfie) camera.
    Front,
}

/// Options for picking an image.
#[derive(Debug, Clone)]
pub struct ImagePickerOptions {
    /// Where to pick the image from.
    pub source: ImageSource,
    /// Maximum width in pixels. If set, the image will be resized.
    pub max_width: Option<f64>,
    /// Maximum height in pixels. If set, the image will be resized.
    pub max_height: Option<f64>,
    /// Image quality (0–100). Only applies to JPEG compression.
    pub image_quality: Option<u8>,
    /// Preferred camera device (only used when `source` is `Camera`).
    pub preferred_camera: CameraDevice,
}

impl Default for ImagePickerOptions {
    fn default() -> Self {
        Self {
            source: ImageSource::Gallery,
            max_width: None,
            max_height: None,
            image_quality: None,
            preferred_camera: CameraDevice::default(),
        }
    }
}

/// A file picked by the user.
#[derive(Debug, Clone)]
pub struct PickedFile {
    /// Absolute file path (or content URI on Android).
    pub path: String,
    /// Display name of the file.
    pub name: String,
}

/// Pick a single image from the gallery or camera.
///
/// Returns `Ok(None)` if the user cancelled.
pub fn pick_image(options: &ImagePickerOptions) -> Result<Option<PickedFile>, String> {
    #[cfg(target_os = "ios")]
    {
        ios::pick_image(options)
    }
    #[cfg(target_os = "android")]
    {
        android::pick_image(options)
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        let _ = options;
        Err("image_picker is only available on iOS and Android".into())
    }
}

/// Pick multiple images from the gallery.
///
/// Returns an empty Vec if the user cancelled.
pub fn pick_multi_image(
    max_width: Option<f64>,
    max_height: Option<f64>,
    image_quality: Option<u8>,
) -> Result<Vec<PickedFile>, String> {
    #[cfg(target_os = "ios")]
    {
        ios::pick_multi_image(max_width, max_height, image_quality)
    }
    #[cfg(target_os = "android")]
    {
        android::pick_multi_image(max_width, max_height, image_quality)
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        let _ = (max_width, max_height, image_quality);
        Err("image_picker is only available on iOS and Android".into())
    }
}

/// Pick a video from the gallery or camera.
///
/// Returns `Ok(None)` if the user cancelled.
pub fn pick_video(
    source: ImageSource,
    preferred_camera: CameraDevice,
) -> Result<Option<PickedFile>, String> {
    #[cfg(target_os = "ios")]
    {
        ios::pick_video(source, preferred_camera)
    }
    #[cfg(target_os = "android")]
    {
        android::pick_video(source, preferred_camera)
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        let _ = (source, preferred_camera);
        Err("image_picker is only available on iOS and Android".into())
    }
}
