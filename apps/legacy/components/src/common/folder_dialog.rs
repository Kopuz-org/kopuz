//! The platform's own folder picker, behind one call for every button that adds
//! a folder.

use std::path::PathBuf;

#[cfg(not(target_os = "android"))]
pub async fn pick_folder() -> Option<PathBuf> {
    rfd::AsyncFileDialog::new()
        .pick_folder()
        .await
        .map(|handle| handle.path().to_path_buf())
}

/// The picked folder is read by path, which needs the shared-media permission,
/// so that is settled before the picker opens rather than failing the scan after.
#[cfg(target_os = "android")]
pub async fn pick_folder() -> Option<PathBuf> {
    use player::systemint;
    if !systemint::has_media_permission() {
        systemint::request_permissions();
        if !systemint::await_media_permission().await {
            return None;
        }
    }
    systemint::pick_folder().await.map(PathBuf::from)
}
