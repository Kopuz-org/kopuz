#[cfg(not(target_arch = "wasm32"))]
pub fn migrate_locations() {
    let Some(dirs) = directories::ProjectDirs::from("com", "temidaradev", "kopuz") else {
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
