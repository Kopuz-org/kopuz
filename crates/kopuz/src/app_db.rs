use std::sync::Arc;

pub static DB_HANDLE: std::sync::OnceLock<db::Db> = std::sync::OnceLock::new();
pub static BOOT_CONFIG: std::sync::OnceLock<config::AppConfig> = std::sync::OnceLock::new();
static REMOTE_API: std::sync::OnceLock<Arc<dyn api::KopuzApi>> = std::sync::OnceLock::new();
static DATABASE_LEASE: std::sync::OnceLock<daemon::DatabaseLease> = std::sync::OnceLock::new();
static STARTUP_ERROR: std::sync::OnceLock<String> = std::sync::OnceLock::new();
#[cfg(not(target_os = "android"))]
static DISCOVERY_GUARD: std::sync::OnceLock<daemon::discovery::DiscoveryGuard> =
    std::sync::OnceLock::new();

#[cfg(not(target_os = "android"))]
pub fn is_embedded() -> bool {
    REMOTE_API.get().is_none()
}

pub fn remote_api() -> Option<Arc<dyn api::KopuzApi>> {
    REMOTE_API.get().cloned()
}

/// A fatal backend-selection failure recorded before the window exists; the
/// app renders it as an error screen instead of starting any daemon service.
pub fn set_startup_error(message: String) {
    let _ = STARTUP_ERROR.set(message);
}

pub fn startup_error() -> Option<&'static str> {
    STARTUP_ERROR.get().map(String::as_str)
}

#[cfg(not(target_os = "android"))]
pub fn select_desktop_backend() -> Result<bool, String> {
    let database_path = db::default_db_path();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("could not start the daemon discovery runtime: {error}"))?;
    let discovered_tracing = rt.block_on(async {
        let discovery_path = daemon::discovery::path();
        if let Some((api, config)) = discovered_api(discovery_path.as_deref()).await? {
            let tracing_enabled = config.tracing_enabled;
            let _ = BOOT_CONFIG.set(config);
            let _ = REMOTE_API.set(api);
            return Ok(Some(tracing_enabled));
        }

        if let Some(path) = discovery_path.as_deref() {
            let guard = match daemon::discovery::DiscoveryGuard::try_claim(path)
                .map_err(|error| format!("could not lock daemon discovery: {error}"))?
            {
                Some(guard) => guard,
                None => {
                    for _ in 0..50 {
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        if let Some((api, config)) = discovered_api(Some(path)).await? {
                            let tracing_enabled = config.tracing_enabled;
                            let _ = BOOT_CONFIG.set(config);
                            let _ = REMOTE_API.set(api);
                            return Ok(Some(tracing_enabled));
                        }
                    }
                    return Err(
                        "another Kopuz daemon is starting but did not become reachable".to_string(),
                    );
                }
            };
            match daemon::discovery::read(path) {
                Some(record) => {
                    let _ = daemon::discovery::remove_record(path, &record);
                }
                None if path.exists() => {
                    let _ = daemon::discovery::remove_invalid(path);
                }
                None => {}
            }
            let _ = DISCOVERY_GUARD.set(guard);
        }

        let lease = daemon::DatabaseLease::try_claim(&database_path)
            .map_err(|error| format!("could not lock the Kopuz database: {error}"))?
            .ok_or_else(|| {
                "another process owns the Kopuz database but exposes no reachable API".to_string()
            })?;
        let _ = DATABASE_LEASE.set(lease);
        Ok::<_, String>(None)
    })?;
    Ok(discovered_tracing.unwrap_or_else(|| {
        db::peek_config(&database_path)
            .map(|config| config.tracing_enabled)
            .unwrap_or(false)
    }))
}

#[cfg(not(target_os = "android"))]
async fn discovered_api(
    path: Option<&std::path::Path>,
) -> Result<Option<(Arc<dyn api::KopuzApi>, config::AppConfig)>, String> {
    let Some(record) = path.and_then(daemon::discovery::read) else {
        return Ok(None);
    };
    if !daemon::discovery::is_serving(&record).await {
        return Ok(None);
    }
    let api: Arc<dyn api::KopuzApi> = Arc::new(
        client::GrpcApi::new(format!("127.0.0.1:{}", record.port), record.token)
            .map_err(|error| format!("could not connect to the Kopuz daemon: {error}"))?,
    );
    let view = api
        .config()
        .await
        .map_err(|error| format!("could not load daemon configuration: {error}"))?;
    let config = serde_json::from_value(view.config)
        .map_err(|error| format!("could not decode daemon configuration: {error}"))?;
    Ok(Some((api, config)))
}

pub fn init_blocking() -> db::Db {
    if DATABASE_LEASE.get().is_none() {
        let lease = daemon::DatabaseLease::try_claim(&db::default_db_path())
            .expect("claim database ownership")
            .expect("another process already owns the Kopuz database");
        let _ = DATABASE_LEASE.set(lease);
    }
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
        match handle.load_config().await {
            Ok(loaded) => {
                let _ = BOOT_CONFIG.set(loaded.unwrap_or_default());
            }
            Err(e) => {
                tracing::error!(error = %e, "kopuz: boot config load failed - seeding defaults");
                let _ = BOOT_CONFIG.set(config::AppConfig::default());
            }
        }
        server::ytmusic::player::init_tier_store(handle.clone());
        utils::db_cache::init(handle.clone());
        handle
    })
}
