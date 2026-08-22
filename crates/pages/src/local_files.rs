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
