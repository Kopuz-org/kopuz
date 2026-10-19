#[cfg(target_os = "linux")]
use std::path::Path;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Readiness {
    Ready,
    Absent,
    Unknown,
}

#[cfg(target_os = "linux")]
fn linux_status(status: libc::c_int) -> Readiness {
    // linux/cdrom.h: CDS_NO_DISC, CDS_TRAY_OPEN, CDS_DISC_OK.
    match status {
        1 | 2 => Readiness::Absent,
        4 => Readiness::Ready,
        _ => Readiness::Unknown,
    }
}

#[cfg(target_os = "linux")]
pub(super) fn readiness(path: &str) -> Readiness {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;

    let Ok(file) = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
    else {
        return Readiness::Unknown;
    };
    // SAFETY: live owned fd, CDROM_DRIVE_STATUS takes an integer slot index
    // (CDSL_CURRENT), not a pointer, and only queries the drive's state.
    linux_status(unsafe { libc::ioctl(file.as_raw_fd(), 0x5326, libc::c_int::MAX) })
}

/// The open handle belongs to this node, even if its persistent symlink is
/// later retargeted or the kernel reuses the same srN name after a reconnect.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Node {
    path: PathBuf,
    #[cfg(target_os = "linux")]
    inode: u64,
}

impl Node {
    pub(super) fn read(path: &str) -> std::io::Result<Self> {
        let path = std::fs::canonicalize(path)?;
        #[cfg(target_os = "linux")]
        let inode = {
            use std::os::unix::fs::MetadataExt;
            std::fs::metadata(&path)?.ino()
        };
        Ok(Self {
            path,
            #[cfg(target_os = "linux")]
            inode,
        })
    }
}

pub(super) fn normalize(paths: Vec<String>) -> Vec<String> {
    #[cfg(target_os = "linux")]
    let paths = persistent_paths(
        paths,
        &[Path::new("/dev/disk/by-id"), Path::new("/dev/disk/by-path")],
    );
    let mut paths = paths;
    paths.sort();
    paths.dedup();
    paths
}

#[cfg(target_os = "linux")]
fn persistent_paths(paths: Vec<String>, directories: &[&Path]) -> Vec<String> {
    let mut aliases = std::collections::HashMap::new();
    for directory in directories {
        let Ok(entries) = std::fs::read_dir(directory) else {
            continue;
        };
        let mut entries: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
        entries.sort();
        for alias in entries {
            if let Ok(target) = std::fs::canonicalize(&alias) {
                // Prefer by-id, then by-path. Ordering must not depend on readdir.
                aliases.entry(target).or_insert(alias);
            }
        }
    }
    paths
        .into_iter()
        .map(|path| {
            let target = std::fs::canonicalize(&path).unwrap_or_else(|_| PathBuf::from(path));
            aliases
                .get(&target)
                .unwrap_or(&target)
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn audio_cd_linux_tray_status_separates_eject_from_busy_and_errors() {
        assert_eq!(linux_status(1), Readiness::Absent);
        assert_eq!(linux_status(2), Readiness::Absent);
        assert_eq!(linux_status(4), Readiness::Ready);
        for status in [0, 3, -1] {
            assert_eq!(linux_status(status), Readiness::Unknown);
        }
    }

    #[test]
    fn audio_cd_device_aliases_and_reconnects_keep_the_same_persistent_path() {
        let root = tempfile::tempdir().unwrap();
        let sr1 = root.path().join("sr1");
        let sr2 = root.path().join("sr2");
        let cdrom = root.path().join("cdrom");
        let by_id = root.path().join("by-id");
        let by_path = root.path().join("by-path");
        std::fs::create_dir(&by_id).unwrap();
        std::fs::create_dir(&by_path).unwrap();
        std::fs::write(&sr1, []).unwrap();
        std::fs::write(&sr2, []).unwrap();
        symlink(&sr1, &cdrom).unwrap();
        let stable = by_id.join("usb-drive-serial");
        symlink(&sr1, &stable).unwrap();
        symlink(&sr1, by_path.join("usb-port")).unwrap();
        let strings = |paths: &[&Path]| {
            paths
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect()
        };
        let mut found = persistent_paths(strings(&[&sr1, &cdrom, &stable]), &[&by_id, &by_path]);
        assert!(found.iter().all(|path| Path::new(path) == stable));
        found.sort();
        found.dedup();
        assert_eq!(found, strings(&[&stable]));
        let before = Node::read(stable.to_str().unwrap()).unwrap();
        std::fs::remove_file(&stable).unwrap();
        symlink(&sr2, &stable).unwrap();
        assert_eq!(
            persistent_paths(strings(&[&sr2]), &[&by_id, &by_path]),
            strings(&[&stable])
        );
        assert_ne!(Node::read(stable.to_str().unwrap()).unwrap(), before);
        // Distinct drives must stay distinct, even when both hold the same CD.
        let other = by_id.join("usb-other-serial");
        symlink(&sr1, &other).unwrap();
        let mut found = persistent_paths(strings(&[&sr1, &sr2]), &[&by_id]);
        found.sort();
        found.dedup();
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn audio_cd_device_falls_back_to_canonical_path_and_detects_reused_nodes() {
        let root = tempfile::tempdir().unwrap();
        let node = root.path().join("sr0");
        let alias = root.path().join("cdrom");
        std::fs::write(&node, []).unwrap();
        symlink(&node, &alias).unwrap();
        let before = Node::read(alias.to_str().unwrap()).unwrap();
        assert_eq!(
            persistent_paths(vec![alias.to_string_lossy().into_owned()], &[]),
            vec![node.to_string_lossy().into_owned()]
        );
        // Keep the old inode alive, as an open device handle would.
        let _handle = std::fs::File::open(&node).unwrap();
        std::fs::remove_file(&node).unwrap();
        std::fs::write(&node, []).unwrap();
        assert_ne!(Node::read(alias.to_str().unwrap()).unwrap(), before);
    }
}
