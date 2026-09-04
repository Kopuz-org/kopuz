/// Process-wide database handle. Opened before the UI mounts, then provided to
/// the app via context.
pub static DB_HANDLE: std::sync::OnceLock<db::Db> = std::sync::OnceLock::new();

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
        db::legacy::migrate_json_store(&handle, &db::config_dir()).await;
        server::ytmusic::player::init_tier_store(handle.clone());
        utils::db_cache::init(handle.clone());
        handle
    })
}
