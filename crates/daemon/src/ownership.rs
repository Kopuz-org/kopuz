//! Exclusive claim on a library database.
//!
//! SQLite takes one writer, and the socket only refuses a second daemon on the
//! same path: `kopuzd --socket /tmp/other` beside a running app would open the
//! same file, restore the same queue and register a second media-key handler.
//! The lock turns that into a named failure at startup instead.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

/// Held for as long as the process owns the database; the advisory lock is
/// released by the kernel on drop, and on exit however the process dies.
pub struct DatabaseLease {
    _file: File,
    path: PathBuf,
}

impl DatabaseLease {
    /// `Ok(None)` means another live process holds it.
    pub fn try_claim(database_path: &Path) -> io::Result<Option<Self>> {
        let path = lock_path(database_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&path)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Self { _file: file, path })),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(error)) => Err(error),
        }
    }

    /// Retry briefly: a relaunch can outrun the previous owner's flush.
    pub async fn claim_with_retry(database_path: &Path) -> io::Result<Option<Self>> {
        for _ in 0..20 {
            if let Some(lease) = Self::try_claim(database_path)? {
                return Ok(Some(lease));
            }
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        }
        Ok(None)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn lock_path(database_path: &Path) -> PathBuf {
    let resolved = std::fs::canonicalize(database_path).unwrap_or_else(|_| {
        let parent = database_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let parent = std::fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
        database_path
            .file_name()
            .map(|name| parent.join(name))
            .unwrap_or(parent)
    });
    let mut path = resolved.as_os_str().to_os_string();
    path.push(".owner.lock");
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lease_is_exclusive_and_released_on_drop() {
        let dir = tempfile::tempdir().expect("tempdir");
        let database = dir.path().join("library.db");
        let first = DatabaseLease::try_claim(&database)
            .expect("first claim")
            .expect("first owner");
        assert!(
            DatabaseLease::try_claim(&database)
                .expect("contending claim")
                .is_none()
        );
        drop(first);
        assert!(
            DatabaseLease::try_claim(&database)
                .expect("claim after drop")
                .is_some()
        );
    }

    #[test]
    fn the_lock_file_sits_beside_the_database() {
        let dir = tempfile::tempdir().expect("tempdir");
        let database = dir.path().join("library.db");
        let lease = DatabaseLease::try_claim(&database)
            .expect("claim")
            .expect("owner");
        assert_eq!(
            lease.path().file_name().and_then(|name| name.to_str()),
            Some("library.db.owner.lock")
        );
    }
}
