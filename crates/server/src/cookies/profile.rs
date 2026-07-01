#[cfg(not(target_os = "windows"))]
use std::path::Path;
use std::path::PathBuf;

/// Directory of an isolated browser profile kopuz owns, named
/// `<prefix>-<server_id>`.
pub fn profile_dir(prefix: &str, server_id: &str) -> PathBuf {
    let safe: String = server_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .collect();
    let leaf = if safe.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}-{safe}")
    };
    directories::ProjectDirs::from("com", "temidaradev", "kopuz")
        .map(|d| {
            // Windows: profiles must live in Local AppData, not Roaming
            // (`config_dir()`) — a OneDrive-synced Roaming profile locks the
            // browser's files and every page hangs.
            #[cfg(target_os = "windows")]
            let base = d.data_local_dir();
            #[cfg(not(target_os = "windows"))]
            let base = d.config_dir();
            base.join(&leaf)
        })
        .unwrap_or_else(|| PathBuf::from(format!("./{leaf}")))
}

pub fn delete_profile(prefix: &str, server_id: &str) -> std::io::Result<()> {
    match std::fs::remove_dir_all(profile_dir(prefix, server_id)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn pick_cookies_path(profile_root: &Path) -> Option<PathBuf> {
    [
        profile_root.join("Default").join("Network").join("Cookies"),
        profile_root.join("Default").join("Cookies"),
    ]
    .into_iter()
    .find(|p| p.exists())
}

pub(crate) fn has_cookie(header: &str, name: &str) -> bool {
    header
        .split(';')
        .any(|p| p.trim().split_once('=').is_some_and(|(k, _)| k == name))
}
