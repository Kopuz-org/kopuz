use crate::android::jni as app_jni;
use std::path::PathBuf;

pub fn temporary_directory() -> Result<PathBuf, String> {
    // Use cache dir + "tmp" subdirectory
    cache_directory().map(|p| p.join("tmp"))
}

pub fn documents_directory() -> Result<PathBuf, String> {
    // AndroidApp::internal_data_path() maps to Context.getFilesDir()
    let app = app_jni::android_app().ok_or_else(|| "AndroidApp not initialised".to_string())?;
    app.internal_data_path()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| "internal_data_path() returned None".into())
}

pub fn cache_directory() -> Result<PathBuf, String> {
    // No direct NDK API for cache dir. Derive from internal_data_path:
    // getFilesDir() = /data/data/<pkg>/files
    // getCacheDir() = /data/data/<pkg>/cache
    let app = app_jni::android_app().ok_or_else(|| "AndroidApp not initialised".to_string())?;
    let files_dir = app
        .internal_data_path()
        .ok_or_else(|| "internal_data_path() returned None".to_string())?;
    // Go up from "files" to the app data root, then into "cache"
    let parent = files_dir
        .parent()
        .ok_or_else(|| "internal_data_path has no parent".to_string())?;
    Ok(parent.join("cache"))
}

pub fn support_directory() -> Result<PathBuf, String> {
    // On Android, getFilesDir() is the closest equivalent to Application Support.
    documents_directory()
}
