use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use fs2::FileExt;

pub struct DatabaseLease {
    _file: File,
}

impl DatabaseLease {
    pub fn try_claim(database_path: &Path) -> io::Result<Option<Self>> {
        let path = lock_path(database_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&path)?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(Some(Self { _file: file })),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(error),
        }
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
}
