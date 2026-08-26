//! The daemon discovery file: `{port, token, pid}` at a well-known path with
//! 0600 permissions, so local frontends can attach without configuration.
//! Written by whichever process serves the API (kopuzd, or the GUI app with
//! its remote control API enabled).

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use api::KopuzApi;
use fs2::FileExt;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryRecord {
    pub port: u16,
    pub token: String,
    pub pid: u32,
}

pub struct DiscoveryLease {
    path: PathBuf,
    token: String,
}

pub struct DiscoveryGuard {
    _file: File,
}

impl DiscoveryGuard {
    pub fn try_claim(discovery_path: &Path) -> io::Result<Option<Self>> {
        let path = guard_path(discovery_path);
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
        let file = options.open(path)?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(Some(Self { _file: file })),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(error),
        }
    }
}

fn guard_path(discovery_path: &Path) -> PathBuf {
    let mut path = discovery_path.as_os_str().to_os_string();
    path.push(".owner.lock");
    PathBuf::from(path)
}

impl DiscoveryLease {
    pub fn claim(path: &Path, port: u16, token: &str) -> std::io::Result<Self> {
        write(path, port, token)?;
        Ok(Self {
            path: path.to_owned(),
            token: token.to_owned(),
        })
    }
}

impl Drop for DiscoveryLease {
    fn drop(&mut self) {
        let _ = remove_owned(&self.path, &self.token);
    }
}

/// The well-known discovery file location: the user runtime dir when the
/// platform has one, the user cache dir otherwise. `None` when neither
/// resolves (an exotic sandbox); serving still works, attaching needs the
/// address by hand.
pub fn path() -> Option<PathBuf> {
    let base = directories::BaseDirs::new()?;
    let dir = base
        .runtime_dir()
        .map(|runtime| runtime.join("kopuz"))
        .unwrap_or_else(|| base.cache_dir().join("kopuz"));
    Some(dir.join("daemon.json"))
}

pub fn random_token() -> String {
    use rand::RngExt;
    let token: u128 = rand::rng().random();
    format!("{token:032x}")
}

pub fn write(path: &Path, port: u16, token: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = DiscoveryRecord {
        port,
        token: token.to_owned(),
        pid: std::process::id(),
    };
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    serde_json::to_writer(&mut file, &body).map_err(std::io::Error::other)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub fn read(path: &Path) -> Option<DiscoveryRecord> {
    let body = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&body).ok()
}

pub async fn is_serving(record: &DiscoveryRecord) -> bool {
    let Ok(api) = client::GrpcApi::new(format!("127.0.0.1:{}", record.port), record.token.clone())
    else {
        return false;
    };
    matches!(
        tokio::time::timeout(Duration::from_millis(800), api.player_state()).await,
        Ok(Ok(_))
    )
}

pub fn remove_record(path: &Path, expected: &DiscoveryRecord) -> std::io::Result<bool> {
    if read(path).as_ref() != Some(expected) {
        return Ok(false);
    }
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

pub fn remove_invalid(path: &Path) -> std::io::Result<bool> {
    if read(path).is_some() {
        return Ok(false);
    }
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

pub fn remove_owned(path: &Path, token: &str) -> std::io::Result<bool> {
    let Some(record) = read(path) else {
        return Ok(false);
    };
    if record.pid != std::process::id() || !constant_time_eq(&record.token, token) {
        return Ok(false);
    }
    remove_record(path, &record)
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    let max = left.len().max(right.len());
    let mut difference = left.len() ^ right.len();
    for index in 0..max {
        difference |= usize::from(
            left.get(index).copied().unwrap_or_default()
                ^ right.get(index).copied().unwrap_or_default(),
        );
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_is_exclusive_and_released_on_drop() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("daemon.json");
        let first = DiscoveryGuard::try_claim(&path)
            .expect("first claim")
            .expect("first owner");
        assert!(
            DiscoveryGuard::try_claim(&path)
                .expect("contending claim")
                .is_none()
        );
        drop(first);
        assert!(
            DiscoveryGuard::try_claim(&path)
                .expect("claim after drop")
                .is_some()
        );
    }
}
