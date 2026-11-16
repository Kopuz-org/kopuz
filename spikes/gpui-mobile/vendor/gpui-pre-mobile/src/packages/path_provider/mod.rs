//! Standard filesystem directory paths.

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "ios")]
mod ios;

use std::path::PathBuf;

/// Returns the temporary directory path.
pub fn temporary_directory() -> Result<PathBuf, String> {
    #[cfg(target_os = "ios")]
    {
        ios::temporary_directory()
    }
    #[cfg(target_os = "android")]
    {
        android::temporary_directory()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        Err("path_provider is only available on iOS and Android".into())
    }
}

/// Returns the documents directory path (user-visible, backed up).
pub fn documents_directory() -> Result<PathBuf, String> {
    #[cfg(target_os = "ios")]
    {
        ios::documents_directory()
    }
    #[cfg(target_os = "android")]
    {
        android::documents_directory()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        Err("path_provider is only available on iOS and Android".into())
    }
}

/// Returns the cache directory path (can be cleared by OS).
pub fn cache_directory() -> Result<PathBuf, String> {
    #[cfg(target_os = "ios")]
    {
        ios::cache_directory()
    }
    #[cfg(target_os = "android")]
    {
        android::cache_directory()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        Err("path_provider is only available on iOS and Android".into())
    }
}

/// Returns the application support directory (not user-visible, backed up).
pub fn support_directory() -> Result<PathBuf, String> {
    #[cfg(target_os = "ios")]
    {
        ios::support_directory()
    }
    #[cfg(target_os = "android")]
    {
        android::support_directory()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        Err("path_provider is only available on iOS and Android".into())
    }
}
