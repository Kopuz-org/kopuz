//! The real kopuz GPUI app on Android: `android_main` starts the daemon core in-process (as the
//! spike does), then runs `kopuz_app` on gpui-pre-mobile's platform, its link attached to that core.

#![cfg(target_os = "android")]

#[path = "../../src/inproc.rs"]
mod inproc;
mod insets;
// `top_inset` is the spike's; the app lays out its own chrome.
#[allow(dead_code)]
#[path = "../../src/lifecycle.rs"]
mod lifecycle;

use gpui_mobile::android::jni;

#[unsafe(no_mangle)]
fn android_main(app: android_activity::AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_filter(
                android_logger::FilterBuilder::new()
                    .parse("info,android_activity::activity_impl=off")
                    .build(),
            )
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

    jni::init_platform(&app);
    let Some(platform) = jni::shared_platform() else {
        log::error!("KOPUZ error gpui-pre-mobile platform missing");
        return;
    };
    // Blocks in gpui-pre-mobile's event loop until the activity is destroyed.
    let application = gpui::Application::with_platform(platform.into_rc());
    kopuz_app::run_in_process(
        application,
        api,
        |cx| insets::Follower::new().install(cx),
        |window, cx| {
            log::info!("KOPUZ main window open");
            lifecycle::Watcher::new().spawn(cx, window);
            insets::Follower::new().spawn(cx, window);
        },
    );
    log::info!("KOPUZ lifecycle exit");
}
