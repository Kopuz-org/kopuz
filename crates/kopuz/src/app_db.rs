/// Process-wide database handle. Opened before the UI mounts, then provided to
/// the app via context.
pub static DB_HANDLE: std::sync::OnceLock<db::Db> = std::sync::OnceLock::new();

#[cfg(not(target_arch = "wasm32"))]
pub fn init_blocking() -> db::Db {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime for db init");
    rt.block_on(async {
        let db_path = db::default_db_path();
        let handle = match db::init(&db_path).await {
            Ok(h) => h,
            Err(e) => {
                let msg = e.to_string().to_lowercase();
                let is_corruption = msg.contains("malformed")
                    || msg.contains("not a database")
                    || msg.contains("corrupt");
                if !is_corruption {
                    panic!(
                        "kopuz database failed to open (not corruption - refusing to discard it): {e}"
                    );
                }
                tracing::error!(error = %e, "kopuz database is corrupt - moving it aside and recreating");
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                for ext in ["", "-wal", "-shm"] {
                    let mut src = db_path.as_os_str().to_os_string();
                    src.push(ext);
                    let mut dst = db_path.as_os_str().to_os_string();
                    dst.push(format!(".corrupt-{ts}{ext}"));
                    let _ = std::fs::rename(src, dst);
                }
                db::init(&db_path).await.expect("recreate kopuz database")
            }
        };
        match handle.import_legacy_json(&db::config_dir()).await {
            Ok(r) if r.ran => tracing::info!(
                tracks = r.tracks,
                albums = r.albums,
                playlists = r.playlists,
                favorites = r.favorites,
                servers = r.servers,
                "kopuz: migrated legacy JSON store into SQLite"
            ),
            Ok(_) => {}
            Err(e) => tracing::error!(error = %e, "kopuz: legacy JSON import failed"),
        }
        if cfg!(debug_assertions) {
            tracing::info!("kopuz: debug build - leaving legacy *.json in place for re-testing");
        } else {
            match handle.finalize_migration(&db::config_dir()).await {
                Ok(n) if n > 0 => {
                    tracing::info!(files = n, "kopuz: legacy *.json renamed to *.json.bak")
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "kopuz: legacy json backup rename failed"),
            }
        }
        server::ytmusic::player::init_tier_store(handle.clone());
        utils::db_cache::init(handle.clone());
        handle
    })
}
