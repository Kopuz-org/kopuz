//! The real kopuz GPUI app on Android: `android_main` starts the daemon core in-process (as the
//! spike does), then runs `kopuz_app` on gpui-pre-mobile's platform, its link attached to that core.

#![cfg(target_os = "android")]

#[path = "../../src/inproc.rs"]
mod inproc;
// `top_inset` is the spike's; the app lays out its own chrome.
#[allow(dead_code)]
#[path = "../../src/lifecycle.rs"]
mod lifecycle;
#[path = "../../src/seed.rs"]
mod seed;

use std::sync::Arc;

use api::{KopuzApi, Page, TrackFilter};
use gpui_mobile::android::jni;

#[unsafe(no_mangle)]
fn android_main(app: android_activity::AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("kopuz-app"),
    );
    jni::install_panic_hook();
    log::info!("KOPUZ start");
    // Before any thread of ours: it sets HOME and TMPDIR.
    inproc::prepare_environment();
    inproc::init_android_tls();
    let api = match inproc::start() {
        Ok(api) => api,
        Err(error) => {
            log::error!("KOPUZ error core: {error}");
            return;
        }
    };
    seed_first_run(api.clone());

    jni::init_platform(&app);
    let Some(platform) = jni::shared_platform() else {
        log::error!("KOPUZ error gpui-pre-mobile platform missing");
        return;
    };
    // Blocks in gpui-pre-mobile's event loop until the activity is destroyed.
    let application = gpui::Application::with_platform(platform.into_rc());
    kopuz_app::run_in_process(application, api, |window, cx| {
        log::info!("KOPUZ main window open");
        lifecycle::Watcher::new().spawn(cx, window);
    });
    log::info!("KOPUZ lifecycle exit");
}

/// An empty library gets the spike's six test tones as a folder source, off the UI thread.
fn seed_first_run(api: Arc<dyn KopuzApi>) {
    let spawned = std::thread::Builder::new()
        .name("kopuz-seed".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => return log::error!("KOPUZ error seed runtime: {error}"),
            };
            runtime.block_on(async move {
                let page = Page {
                    offset: 0,
                    limit: 1,
                };
                match api.tracks(TrackFilter::default(), page).await {
                    Ok(tracks) if tracks.total > 0 => {
                        log::info!("KOPUZ library tracks={}", tracks.total);
                    }
                    Ok(_) => match seed::seed(&*api).await {
                        Ok(total) => log::info!("KOPUZ seeded tracks={total}"),
                        Err(error) => log::error!("KOPUZ error seed: {}", error.message),
                    },
                    Err(error) => log::error!("KOPUZ error tracks: {}", error.message),
                }
            });
        });
    if let Err(error) = spawned {
        log::error!("KOPUZ error seed thread: {error}");
    }
}
