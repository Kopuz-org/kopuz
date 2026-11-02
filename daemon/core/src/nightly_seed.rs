//! Kopuz Nightly's first start: a copy of the stable app's library, settings and sign-ins, so trying it needs no setup.

use std::path::{Path, PathBuf};

use config::flavor::Flavor;

const DATABASE: &str = "kopuz.db";

/// Seed Kopuz Nightly from the stable app once; anything that goes wrong leaves it to start empty.
pub async fn seed_from_stable() {
    if Flavor::current() != Flavor::Nightly {
        return;
    }
    let (Some(nightly), Some(stable)) = (
        Flavor::Nightly.project_dirs(),
        Flavor::Stable.project_dirs(),
    ) else {
        return;
    };
    if nightly.config_dir().exists() || !stable.config_dir().exists() {
        return;
    }
    tracing::info!(from = %stable.config_dir().display(), "copying the stable library into Kopuz Nightly");
    let pairs = distinct_pairs(&[
        (stable.config_dir(), nightly.config_dir()),
        (stable.data_dir(), nightly.data_dir()),
        (stable.data_local_dir(), nightly.data_local_dir()),
    ]);
    // Only what this seed creates is undone; a directory already there is Nightly's own.
    let created: Vec<PathBuf> = pairs
        .iter()
        .map(|(_, to)| to.clone())
        .filter(|to| !to.exists())
        .collect();
    let copied = tokio::task::spawn_blocking(move || {
        pairs.iter().try_for_each(|(from, to)| copy_tree(from, to))
    })
    .await;
    let mut seeded = match copied {
        Ok(Ok(())) => true,
        Ok(Err(error)) => {
            tracing::warn!(%error, "could not copy all of the stable app's files");
            false
        }
        Err(error) => {
            tracing::warn!(%error, "the copy of the stable app's files stopped");
            false
        }
    };
    let (from, to) = (
        stable.config_dir().join(DATABASE),
        nightly.config_dir().join(DATABASE),
    );
    if seeded
        && from.exists()
        && let Err(error) = db::snapshot(&from, &to).await
    {
        tracing::warn!(%error, "could not copy the stable library");
        seeded = false;
    }
    // Half a copy would pass for a seeded Nightly on every later start, so it goes and Nightly starts empty.
    if !seeded {
        tracing::warn!("Kopuz Nightly starts empty");
        for dir in &created {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// On Linux the data and local-data directories are one, so each is copied once.
fn distinct_pairs(pairs: &[(&Path, &Path)]) -> Vec<(PathBuf, PathBuf)> {
    let mut seen: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (from, to) in pairs {
        if !seen.iter().any(|(already, _)| already == from) {
            seen.push((from.to_path_buf(), to.to_path_buf()));
        }
    }
    seen
}

/// The live database is snapshotted instead, its lock belongs to the stable daemon, and caches rebuild themselves.
fn skipped(name: &str) -> bool {
    name.starts_with("kopuz.db")
        || name.starts_with("kopuz-debug.db")
        || name.ends_with(".owner.lock")
        || name.starts_with("offline-cache")
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    if !from.is_dir() {
        return Ok(());
    }
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if skipped(&name.to_string_lossy()) {
            continue;
        }
        let target = to.join(&name);
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_live_database_its_lock_and_the_song_cache_are_not_copied() {
        for name in [
            "kopuz.db",
            "kopuz.db-wal",
            "kopuz.db-shm",
            "kopuz.db.owner.lock",
            "kopuz-debug.db",
            "offline-cache",
            "offline-cache-debug",
        ] {
            assert!(skipped(name), "{name} should be skipped");
        }
        for name in [
            "settings.toml",
            "settings.d",
            "browser-profile",
            "artwork-uploads",
        ] {
            assert!(!skipped(name), "{name} should be copied");
        }
    }

    #[test]
    fn a_tree_is_copied_without_the_skipped_entries() {
        let from = tempfile::tempdir().unwrap();
        let to = tempfile::tempdir().unwrap();
        std::fs::write(from.path().join("settings.toml"), "a = 1").unwrap();
        std::fs::write(from.path().join("kopuz.db.owner.lock"), "").unwrap();
        std::fs::create_dir_all(from.path().join("profile/Default")).unwrap();
        std::fs::write(from.path().join("profile/Default/Cookies"), "c").unwrap();
        std::fs::create_dir_all(from.path().join("offline-cache")).unwrap();
        let target = to.path().join("nightly");

        copy_tree(from.path(), &target).unwrap();

        assert!(target.join("settings.toml").is_file());
        assert!(target.join("profile/Default/Cookies").is_file());
        assert!(!target.join("kopuz.db.owner.lock").exists());
        assert!(!target.join("offline-cache").exists());
    }

    #[test]
    fn a_shared_data_directory_is_copied_once() {
        let (a, b, c) = (Path::new("/c"), Path::new("/d"), Path::new("/n"));
        let pairs = distinct_pairs(&[(a, c), (b, c), (b, c)]);
        assert_eq!(pairs.len(), 2);
    }
}
