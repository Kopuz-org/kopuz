use std::sync::Arc;

pub static DB_HANDLE: std::sync::OnceLock<db::Db> = std::sync::OnceLock::new();
pub static BOOT_CONFIG: std::sync::OnceLock<config::AppConfig> = std::sync::OnceLock::new();
/// Address and bearer token of the daemon this process attached to. Only the
/// endpoint is kept: the client itself is built on the runtime that will use it
/// (see [`remote_api`]).
#[cfg(not(target_os = "android"))]
static REMOTE_ENDPOINT: std::sync::OnceLock<(String, String)> = std::sync::OnceLock::new();
static REMOTE_API: std::sync::OnceLock<Arc<dyn api::KopuzApi>> = std::sync::OnceLock::new();
static DATABASE_LEASE: std::sync::OnceLock<daemon::DatabaseLease> = std::sync::OnceLock::new();
static STARTUP_ERROR: std::sync::OnceLock<String> = std::sync::OnceLock::new();
#[cfg(not(target_os = "android"))]
static DISCOVERY_GUARD: std::sync::OnceLock<daemon::discovery::DiscoveryGuard> =
    std::sync::OnceLock::new();

#[cfg(not(target_os = "android"))]
pub fn is_embedded() -> bool {
    REMOTE_ENDPOINT.get().is_none()
}

/// The attached daemon's client, built on first use and cached.
///
/// Tonic spawns a lazy channel's connection worker onto whichever runtime
/// constructs it, so the client must not be built during discovery: that
/// runtime is temporary, and its drop would abort the worker and leave every
/// later RPC failing on a dead channel (issue #661). Call this from the
/// long-lived runtime that owns the app.
pub fn remote_api() -> Option<Arc<dyn api::KopuzApi>> {
    if let Some(api) = REMOTE_API.get() {
        return Some(api.clone());
    }
    connect_remote_api()
}

/// Android always embeds the daemon, and the gRPC client is a desktop-only
/// dependency.
#[cfg(target_os = "android")]
fn connect_remote_api() -> Option<Arc<dyn api::KopuzApi>> {
    None
}

#[cfg(not(target_os = "android"))]
fn connect_remote_api() -> Option<Arc<dyn api::KopuzApi>> {
    let (addr, token) = REMOTE_ENDPOINT.get()?;
    match client::GrpcApi::new(addr.clone(), token.clone()) {
        Ok(api) => {
            let _ = REMOTE_API.set(Arc::new(api));
            REMOTE_API.get().cloned()
        }
        Err(error) => {
            tracing::error!(%error, %addr, "could not build the Kopuz daemon client");
            None
        }
    }
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
        if let Some((endpoint, config)) = discovered_api(discovery_path.as_deref()).await? {
            let tracing_enabled = config.tracing_enabled;
            let _ = BOOT_CONFIG.set(config);
            let _ = REMOTE_ENDPOINT.set(endpoint);
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
                        if let Some((endpoint, config)) = discovered_api(Some(path)).await? {
                            let tracing_enabled = config.tracing_enabled;
                            let _ = BOOT_CONFIG.set(config);
                            let _ = REMOTE_ENDPOINT.set(endpoint);
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

/// Probes the discovered daemon and reads its configuration, returning the
/// endpoint to reconnect to rather than the client that did the probing: this
/// runs on a throwaway runtime, and a tonic channel does not outlive the
/// runtime it was built on.
#[cfg(not(target_os = "android"))]
async fn discovered_api(
    path: Option<&std::path::Path>,
) -> Result<Option<((String, String), config::AppConfig)>, String> {
    use api::KopuzApi as _;

    let Some(record) = path.and_then(daemon::discovery::read) else {
        return Ok(None);
    };
    if !daemon::discovery::is_serving(&record).await {
        return Ok(None);
    }
    let endpoint = (format!("127.0.0.1:{}", record.port), record.token);
    let api = client::GrpcApi::new(endpoint.0.clone(), endpoint.1.clone())
        .map_err(|error| format!("could not connect to the Kopuz daemon: {error}"))?;
    let view = api
        .config()
        .await
        .map_err(|error| format!("could not load daemon configuration: {error}"))?;
    let config = serde_json::from_value(view.config)
        .map_err(|error| format!("could not decode daemon configuration: {error}"))?;
    Ok(Some((endpoint, config)))
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
