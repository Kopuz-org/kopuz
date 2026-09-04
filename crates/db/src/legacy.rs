use std::path::Path;

/// The ProjectDirs triple used through 0.15.1.
const LEGACY_IDENTITY: (&str, &str, &str) = ("com", "temidaradev", "kopuz");

/// Move everything written under the pre-rename identity into the current one.
///
/// macOS and Windows derive the directory name from the qualifier and
/// organization, so the rename to `moe.kopuz.kopuz` points the app at an empty
/// directory and every library, playlist and setting looks lost. Linux ignores
/// both (XDG keys off the application name alone), so there source and
/// destination are the same path and every move below is skipped.
#[must_use]
pub fn migrate_identity() -> Vec<String> {
    let (Some(old), Some(new)) = (
        directories::ProjectDirs::from(LEGACY_IDENTITY.0, LEGACY_IDENTITY.1, LEGACY_IDENTITY.2),
        directories::ProjectDirs::from("moe", "kopuz", "kopuz"),
    ) else {
        return Vec::new();
    };
    [
        (old.config_dir(), new.config_dir()),
        (old.data_dir(), new.data_dir()),
        (old.data_local_dir(), new.data_local_dir()),
        (old.cache_dir(), new.cache_dir()),
    ]
    .into_iter()
    .filter_map(|(from, to)| move_dir(from, to))
    .collect()
}

/// Rename `from` onto `to`, unless the move would be ambiguous: an existing
/// destination means this build already owns that directory, and merging the
/// two could restore a stale database over a live one.
fn move_dir(from: &Path, to: &Path) -> Option<String> {
    if from == to || !from.is_dir() || to.exists() {
        return None;
    }
    if let Some(parent) = to.parent()
        && let Err(error) = std::fs::create_dir_all(parent)
    {
        return Some(format!("cannot create {}: {error}", parent.display()));
    }
    match std::fs::rename(from, to) {
        Ok(()) => Some(format!("moved {} to {}", from.display(), to.display())),
        Err(error) => Some(format!(
            "cannot move {} to {}: {error}; the previous version's data is still there",
            from.display(),
            to.display()
        )),
    }
}

/// Rewrite an absolute path stored under the pre-rename identity onto the
/// current directory layout.
pub fn remap_identity_path(path: &str) -> Option<String> {
    if Path::new(path).exists() {
        return None;
    }
    let (old, new) = (
        directories::ProjectDirs::from(LEGACY_IDENTITY.0, LEGACY_IDENTITY.1, LEGACY_IDENTITY.2)?,
        directories::ProjectDirs::from("moe", "kopuz", "kopuz")?,
    );
    let pairs = [
        (old.config_dir(), new.config_dir()),
        (old.data_dir(), new.data_dir()),
        (old.data_local_dir(), new.data_local_dir()),
        (old.cache_dir(), new.cache_dir()),
    ];
    remap_across(path, &pairs)
}

fn remap_across(path: &str, pairs: &[(&Path, &Path)]) -> Option<String> {
    for (from, to) in pairs {
        if let Ok(rest) = Path::new(path).strip_prefix(from) {
            let candidate = to.join(rest);
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
    }
    None
}

/// Move legacy JSON stores from the old cache location into the config
/// directory before the one-shot SQLite import runs.
pub fn migrate_locations() {
    let Some(dirs) = directories::ProjectDirs::from("moe", "kopuz", "kopuz") else {
        return;
    };
    let new_config = dirs.config_dir().to_path_buf();
    let sentinel = new_config.join(".migrated");
    if sentinel.exists() {
        return;
    }

    if let Err(error) = std::fs::create_dir_all(&new_config) {
        tracing::warn!(%error, path = %new_config.display(), "legacy config directory creation failed");
        return;
    }

    let old_cache = dirs.cache_dir().to_path_buf();
    let files = [
        "library.json",
        "playlists.json",
        "favorites.json",
        "queue_state.json",
    ];
    let mut all_moved = true;
    for file in files {
        let source = old_cache.join(file);
        let destination = new_config.join(file);
        if source.exists() && !destination.exists() {
            if let Err(error) = std::fs::rename(&source, &destination) {
                all_moved = false;
                tracing::warn!(%error, file, "legacy store location migration failed");
            } else {
                tracing::info!(file, "legacy store moved to the config directory");
            }
        }
    }

    if !all_moved {
        return;
    }
    if let Err(error) = std::fs::write(&sentinel, "") {
        tracing::warn!(%error, path = %sentinel.display(), "legacy location sentinel write failed");
    }
}

/// Import the legacy JSON stores once, then retire them outside debug builds.
pub async fn migrate_json_store(database: &crate::Db, config_dir: &Path) {
    match database.import_legacy_json(config_dir).await {
        Ok(report) if report.ran => tracing::info!(
            tracks = report.tracks,
            albums = report.albums,
            playlists = report.playlists,
            favorites = report.favorites,
            servers = report.servers,
            "migrated legacy JSON store into SQLite"
        ),
        Ok(_) => {}
        Err(error) => {
            tracing::error!(%error, "legacy JSON import failed");
            return;
        }
    }
    if cfg!(debug_assertions) {
        tracing::info!("debug build: leaving legacy JSON stores in place");
        return;
    }
    match database.finalize_migration(config_dir).await {
        Ok(files) if files > 0 => {
            tracing::info!(files, "legacy JSON stores renamed with .bak suffix")
        }
        Ok(_) => {}
        Err(error) => tracing::warn!(%error, "legacy JSON backup rename failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::{move_dir, remap_across};
    use std::fs;

    fn tmp(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("kopuz-identity-{name}"));
        let _ = fs::remove_dir_all(&path);
        path
    }

    #[test]
    fn moves_when_destination_is_free() {
        let root = tmp("free");
        let (from, to) = (root.join("old"), root.join("new"));
        fs::create_dir_all(&from).unwrap();
        fs::write(from.join("kopuz.db"), b"library").unwrap();

        assert!(move_dir(&from, &to).is_some());
        assert_eq!(fs::read(to.join("kopuz.db")).unwrap(), b"library");
        assert!(!from.exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn leaves_both_alone_when_destination_exists() {
        let root = tmp("occupied");
        let (from, to) = (root.join("old"), root.join("new"));
        fs::create_dir_all(&from).unwrap();
        fs::create_dir_all(&to).unwrap();
        fs::write(from.join("kopuz.db"), b"stale").unwrap();
        fs::write(to.join("kopuz.db"), b"live").unwrap();

        assert!(move_dir(&from, &to).is_none());
        assert_eq!(fs::read(to.join("kopuz.db")).unwrap(), b"live");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn remap_rewrites_only_paths_that_exist_at_the_new_root() {
        let root = tmp("remap");
        let (old_cache, new_cache) = (root.join("old-cache"), root.join("new-cache"));
        fs::create_dir_all(new_cache.join("covers")).unwrap();
        fs::write(new_cache.join("covers/a.jpg"), b"img").unwrap();
        let pairs = [(old_cache.as_path(), new_cache.as_path())];

        let stored = old_cache.join("covers/a.jpg");
        let remapped = remap_across(&stored.to_string_lossy(), &pairs);
        assert_eq!(remapped.as_deref(), new_cache.join("covers/a.jpg").to_str());

        let missing = old_cache.join("covers/b.jpg");
        assert!(remap_across(&missing.to_string_lossy(), &pairs).is_none());

        let unrelated = root.join("elsewhere/c.jpg");
        assert!(remap_across(&unrelated.to_string_lossy(), &pairs).is_none());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn same_path_is_a_noop() {
        let root = tmp("same");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("kopuz.db"), b"library").unwrap();

        assert!(move_dir(&root, &root).is_none());
        assert_eq!(fs::read(root.join("kopuz.db")).unwrap(), b"library");
        let _ = fs::remove_dir_all(&root);
    }
}
