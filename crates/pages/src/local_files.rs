//! Filesystem policy for destructive local-library actions.

use std::path::{Path, PathBuf};

use config::{AppConfig, Source};

fn configured_roots<'a>(config: &'a AppConfig, source: &Source) -> &'a [PathBuf] {
    match source {
        Source::Local => &config.music_directory,
        Source::LocalLibrary(id) => config
            .local_sources
            .iter()
            .find(|saved| saved.id == *id)
            .map_or(&[], |saved| saved.directories.as_slice()),
        Source::Server(_) => &[],
    }
}

fn is_inside_configured_root(config: &AppConfig, source: &Source, path: &Path) -> bool {
    let Ok(target) = std::fs::canonicalize(path) else {
        return false;
    };
    configured_roots(config, source).iter().any(|root| {
        std::fs::canonicalize(root).is_ok_and(|root| target != root && target.starts_with(root))
    })
}

/// Remove a local track only when its canonical path is inside the active library.
pub(crate) fn remove(config: &AppConfig, source: &Source, path: &Path) -> std::io::Result<bool> {
    if !source.is_local() || !is_inside_configured_root(config, source, path) {
        tracing::warn!(
            source = %source.as_str(),
            path = %path.display(),
            "refusing to delete a track outside the configured library roots"
        );
        return Ok(false);
    }
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// Whether `path` is settled enough that the track's rows can go.
///
/// [`remove`] answers `Ok(false)` both for a file that was already gone and for
/// one it refused to touch because it sits outside the library roots, and those
/// pull in opposite directions: the first is a half-finished delete the rows
/// should now follow, the second is a live file the rows must keep pointing at.
/// Re-checking the path separates them.
///
/// Failing closed is the recoverable direction. Rows that outlive their file
/// leave the item on screen, so deleting it again retries; rows deleted out from
/// under a surviving file leave it playing from nothing and unreachable.
pub(crate) fn cleared(config: &AppConfig, source: &Source, path: &Path) -> bool {
    match remove(config, source, path) {
        Ok(true) => true,
        // `symlink_metadata`, never `exists`: that follows the link, so a
        // dangling symlink reads as already gone while the link itself is still
        // on disk and still the thing the rows point at. Only a path that is
        // genuinely absent clears; anything we cannot answer for fails closed.
        Ok(false) => match std::fs::symlink_metadata(path) {
            Ok(_) => false,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
            Err(error) => {
                tracing::warn!(%error, path = %path.display(), "delete: could not tell whether the file is gone");
                false
            }
        },
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "delete: removing the file failed");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deletes_only_files_inside_the_selected_library() {
        let directory = tempfile::tempdir().expect("temporary library");
        let library = directory.path().join("library");
        std::fs::create_dir(&library).expect("create library");
        let inside = library.join("inside.mp3");
        let outside = directory.path().join("outside.mp3");
        std::fs::write(&inside, b"inside").expect("write inside file");
        std::fs::write(&outside, b"outside").expect("write outside file");
        let config = AppConfig {
            music_directory: vec![library],
            ..AppConfig::default()
        };

        assert!(!remove(&config, &Source::Local, &outside).expect("safe rejection"));
        assert!(outside.exists());
        assert!(remove(&config, &Source::Local, &inside).expect("delete inside file"));
        assert!(!inside.exists());
    }

    #[test]
    fn a_path_that_is_already_gone_clears_its_rows() {
        let directory = tempfile::tempdir().expect("temporary library");
        let library = directory.path().join("library");
        std::fs::create_dir(&library).expect("create library");
        let missing = library.join("missing.mp3");
        let config = AppConfig {
            music_directory: vec![library],
            ..AppConfig::default()
        };

        // The half-finished delete: the file went, the rows did not. A retry
        // has to get past this point or they are stranded for good.
        assert!(cleared(&config, &Source::Local, &missing));
    }

    #[cfg(unix)]
    #[test]
    fn a_dangling_symlink_does_not_clear_its_rows() {
        let directory = tempfile::tempdir().expect("temporary library");
        let library = directory.path().join("library");
        std::fs::create_dir(&library).expect("create library");
        let link = library.join("dangling.mp3");
        std::os::unix::fs::symlink(directory.path().join("missing.mp3"), &link)
            .expect("create dangling symlink");
        let config = AppConfig {
            music_directory: vec![library],
            ..AppConfig::default()
        };

        // `exists` follows the link and reports it gone, which would drop the
        // rows while the link is still sitting in the library.
        assert!(!link.exists());
        assert!(std::fs::symlink_metadata(&link).is_ok());
        assert!(!cleared(&config, &Source::Local, &link));
        assert!(std::fs::symlink_metadata(&link).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_that_resolves_outside_the_library() {
        let directory = tempfile::tempdir().expect("temporary library");
        let library = directory.path().join("library");
        std::fs::create_dir(&library).expect("create library");
        let outside = directory.path().join("outside.mp3");
        std::fs::write(&outside, b"outside").expect("write outside file");
        let link = library.join("link.mp3");
        std::os::unix::fs::symlink(&outside, &link).expect("create symlink");
        let config = AppConfig {
            music_directory: vec![library],
            ..AppConfig::default()
        };

        assert!(!remove(&config, &Source::Local, &link).expect("safe rejection"));
        assert!(outside.exists());
        assert!(link.exists());
    }
}
