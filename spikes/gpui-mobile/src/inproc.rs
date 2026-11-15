//! The kopuz daemon core, hosted in this process the way the legacy Android
//! app does it (`apps/legacy/kopuz/src/backend.rs`): `daemon::boot::assemble`
//! on its own multi-thread runtime, on its own thread, for the life of the
//! process. The UI reaches it through the core's in-process `LocalApi`, which
//! implements `api::KopuzApi`; no socket is involved.
//!
//! Everything the core writes lives under the app's private files directory
//! (`Context.getFilesDir()`, which `db::config_dir()` asks for over JNI
//! through ndk-context; android-activity initialises ndk-context before
//! `android_main`): the database `kopuz.db`, `settings.toml` and `plugins/`
//! beside it, the offline cache, and (through `HOME`, see
//! [`prepare_environment`]) the cover, artwork and download caches.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, mpsc};

use api::KopuzApi;
use kopuz_core::boot::{Core, CoreArgs};

static CORE: OnceLock<Core> = OnceLock::new();

/// The app's private files directory (`Context.getFilesDir()` on Android).
pub fn files_dir() -> PathBuf {
    db::config_dir()
}

/// Point `HOME` and `TMPDIR` into the files directory, before any thread of
/// ours exists. The core places its caches under `directories`' XDG layout,
/// which resolves from `HOME`; an app process has no writable home, and
/// `std::env::temp_dir()` on Android is `/data/local/tmp`, which apps cannot
/// write either. Returns the files directory.
pub fn prepare_environment() -> PathBuf {
    let dir = files_dir();
    let tmp = dir.join("tmp");
    if let Err(error) = std::fs::create_dir_all(&tmp) {
        log::info!("SPIKE error files dir {}: {error}", dir.display());
    }
    // SAFETY: called first thing in `android_main`, before the spike starts
    // any thread; nothing else in the process reads the C environment then.
    unsafe {
        std::env::set_var("HOME", &dir);
        std::env::set_var("TMPDIR", &tmp);
    }
    log::info!("SPIKE files_dir={}", dir.display());
    dir
}

/// Start the core once and hand out its API. Blocks until the library is
/// open; a failed start leaves nothing behind, so the caller may retry.
pub fn start() -> Result<Arc<dyn KopuzApi>, String> {
    static STARTING: Mutex<()> = Mutex::new(());
    let _starting = STARTING.lock().unwrap_or_else(|poison| poison.into_inner());
    if let Some(core) = CORE.get() {
        let api: Arc<dyn KopuzApi> = core.api.clone();
        return Ok(api);
    }

    let dir = files_dir();
    std::fs::create_dir_all(&dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    let db_path = dir.join("kopuz.db");
    log::info!("SPIKE core starting db={}", db_path.display());
    let args = CoreArgs {
        db_path: Some(db_path.to_string_lossy().into_owned()),
        ..CoreArgs::default()
    };

    let (ready_tx, ready_rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("kopuz-core".into())
        .spawn(move || {
            kopuz_core::boot::prepare_thread();
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .thread_name("kopuz-core-rt")
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    let _ = ready_tx.send(Err(format!("core runtime: {error}")));
                    return;
                }
            };
            runtime.block_on(async move {
                match kopuz_core::boot::assemble(&args).await {
                    Ok(core) => {
                        let _ = CORE.set(core);
                        let _ = ready_tx.send(Ok(()));
                        // Every service the core spawned lives on this runtime.
                        std::future::pending::<()>().await;
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error.to_string()));
                    }
                }
            });
            runtime.shutdown_background();
        })
        .map_err(|error| format!("core thread: {error}"))?;

    match ready_rx.recv() {
        Ok(Ok(())) => {
            let core = CORE.get().ok_or("core vanished")?;
            log::info!("SPIKE core ready");
            let api: Arc<dyn KopuzApi> = core.api.clone();
            Ok(api)
        }
        Ok(Err(error)) => Err(error),
        Err(_) => Err("the core thread stopped before it was ready".into()),
    }
}

/// Hand the Android trust store to rustls before anything opens a TLS
/// connection, as `apps/legacy/kopuz/src/main.rs` does: rustls-platform-verifier
/// panics on first verification if it never got a JVM and Context. The spike's
/// APK carries no Java code, so verification itself still fails (as an
/// error) without the verifier's Kotlin class; local folders need no TLS.
#[cfg(target_os = "android")]
pub fn init_android_tls() {
    let ctx = ndk_context::android_context();
    if ctx.vm().is_null() || ctx.context().is_null() {
        log::info!("SPIKE error tls: no JVM or Context on the ndk context");
        return;
    }
    // SAFETY: android-activity populates ndk_context before `android_main`
    // runs, and both handles stay valid for the lifetime of the process.
    let vm = unsafe { ::jni::JavaVM::from_raw(ctx.vm().cast()) };
    let raw_context = ctx.context().cast();
    let result = vm.attach_current_thread(|env| {
        // SAFETY: `raw_context` is the Activity's global reference.
        let context = unsafe { ::jni::objects::JObject::from_raw(env, raw_context) };
        rustls_platform_verifier::android::init_with_env(env, context)
    });
    if let Err(error) = result {
        let error: ::jni::errors::Error = error;
        log::info!("SPIKE error tls: {error}");
    }
}
