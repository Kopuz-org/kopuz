use std::path::PathBuf;

/// Mirrors `kopuzd::default_socket_path`, which sits on the daemon side of the wall.
#[cfg(not(windows))]
pub fn default_socket_path() -> Option<PathBuf> {
    let base = directories::BaseDirs::new()?;
    let dir = base
        .runtime_dir()
        .map(|runtime| runtime.join("kopuz"))
        .unwrap_or_else(|| base.cache_dir().join("kopuz"));
    Some(dir.join("kopuzd.sock"))
}

#[cfg(windows)]
pub fn default_socket_path() -> Option<PathBuf> {
    None
}
