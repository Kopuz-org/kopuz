//! The folders a plugin with folder access reads, as it sees them. Each is
//! mounted read-only at a path of its own: the same path on Unix, and on
//! Windows `C:\Music` at `/C:/Music`, since WASI paths are rooted and
//! separated by slashes. A path the plugin hands back is turned into the
//! host's file here, and only if it lies in one of the folders.

use std::path::{Component, Path, PathBuf};

/// Where the host folder `host` is mounted in the sandbox.
pub fn guest_path(host: &str) -> String {
    let slashed = host.replace('\\', "/");
    let trimmed = slashed.trim_end_matches('/');
    match trimmed {
        "" => "/".to_string(),
        rooted if rooted.starts_with('/') => rooted.to_string(),
        relative => format!("/{relative}"),
    }
}

/// The file on the host that `guest` names, if it lies in one of the host
/// `folders`. Checked by its words alone; a caller that opens the file still
/// resolves links and checks where it really is.
pub fn host_path(guest: &str, folders: &[String]) -> Option<PathBuf> {
    folders.iter().find_map(|folder| {
        let root = guest_path(folder);
        let rest = if root == "/" {
            guest.strip_prefix('/')?
        } else {
            let rest = guest.strip_prefix(root.as_str())?;
            if rest.is_empty() {
                rest
            } else {
                rest.strip_prefix('/')?
            }
        };
        let mut path = PathBuf::from(folder);
        for part in rest.split('/').filter(|part| !part.is_empty()) {
            let mut parts = Path::new(part).components();
            match (parts.next(), parts.next()) {
                (Some(Component::Normal(name)), None) => path.push(name),
                _ => return None,
            }
        }
        Some(path)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_is_mounted_at_a_rooted_slash_path() {
        assert_eq!(guest_path("/home/a/osu"), "/home/a/osu");
        assert_eq!(guest_path("/home/a/osu/"), "/home/a/osu");
        assert_eq!(guest_path(r"C:\Users\a\osu!"), "/C:/Users/a/osu!");
        assert_eq!(guest_path("/"), "/");
    }

    #[test]
    fn a_guest_path_maps_back_only_inside_a_folder() {
        let folders = ["/home/a/osu".to_string()];
        assert_eq!(
            host_path("/home/a/osu/files/a/ab/abc", &folders),
            Some(PathBuf::from("/home/a/osu/files/a/ab/abc"))
        );
        assert_eq!(
            host_path("/home/a/osu", &folders),
            Some(PathBuf::from("/home/a/osu"))
        );
        for outside in [
            "/home/a/osu2/x",
            "/home/a/osu/../.ssh/id_ed25519",
            "/home/a/osu/./x",
            "/etc/passwd",
            "home/a/osu/x",
        ] {
            assert_eq!(host_path(outside, &folders), None, "{outside}");
        }
    }

    #[test]
    fn a_windows_folder_maps_back_to_its_own_path() {
        let folders = [r"C:\Games\osu!".to_string()];
        let mapped = host_path("/C:/Games/osu!/Songs/1 A - B/audio.mp3", &folders);
        let expected = ["Songs", "1 A - B", "audio.mp3"]
            .iter()
            .fold(PathBuf::from(r"C:\Games\osu!"), |path, part| {
                path.join(part)
            });
        assert_eq!(mapped, Some(expected));
    }
}
