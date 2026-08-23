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
///
/// Runs before the tracing subscriber exists, because [`db::peek_config`] reads
/// the database during subscriber setup and would otherwise create a fresh one
/// at the new location. The outcome is returned rather than logged for that
/// reason: a migration that failed silently would present as a lost library.
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
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        return Some(format!("cannot create {}: {e}", parent.display()));
    }
    match std::fs::rename(from, to) {
        Ok(()) => Some(format!("moved {} to {}", from.display(), to.display())),
        Err(e) => Some(format!(
            "cannot move {} to {}: {e}; the previous version's data is still there",
            from.display(),
            to.display()
        )),
    }
}

pub fn migrate_locations() {
    let Some(dirs) = directories::ProjectDirs::from("moe", "kopuz", "kopuz") else {
        return;
    };
    let new_config = dirs.config_dir().to_path_buf();
    let sentinel = new_config.join(".migrated");
    if sentinel.exists() {
        return;
    }

    let old_cache = dirs.cache_dir().to_path_buf();
    let files = [
        "library.json",
        "playlists.json",
        "favorites.json",
        "queue_state.json",
    ];
    for file in files {
        let src = old_cache.join(file);
        let dst = new_config.join(file);
        if src.exists() && !dst.exists() {
            if let Err(e) = std::fs::rename(&src, &dst) {
                tracing::warn!("Failed to migrate {file} from cache to config: {e}");
            } else {
                tracing::info!("Migrated {file} to config dir");
            }
        }
    }

    let _ = std::fs::write(&sentinel, "");
}

#[cfg(test)]
mod tests {
    use super::move_dir;
    use std::fs;

    fn tmp(name: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("kopuz-identity-{name}"));
        let _ = fs::remove_dir_all(&p);
        p
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

    /// The Linux case: both identities resolve to the same directory, so the
    /// move must not run.
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
