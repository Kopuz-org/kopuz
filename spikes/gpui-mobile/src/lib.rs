//! kopuz GPUI-on-Android spike: one screen (library list + play) rendered by
//! gpui-pre on gpui-pre-mobile's Android platform (wgpu, Vulkan first), fed
//! by the kopuz daemon core running in-process, or by a kopuzd over TCP when
//! the system property `debug.kopuz.spike.addr` names one.
//! `android_main` is the native-activity entry point.

mod daemon;
mod inproc;
mod seed;
mod ui;

#[cfg(target_os = "android")]
mod lifecycle;

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: android_activity::AndroidApp) {
    use gpui::{App, AppContext as _, Application, WindowOptions};
    use gpui_mobile::android::jni;

    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("kopuz-spike"),
    );
    jni::install_panic_hook();
    log::info!("SPIKE start");
    // Before any thread of ours: it sets HOME and TMPDIR.
    inproc::prepare_environment();
    inproc::init_android_tls();
    let backend = daemon::Backend::from_environment();
    log::info!("SPIKE backend={backend}");

    let handle = match daemon::start_runtime() {
        Ok(handle) => handle,
        Err(error) => {
            log::info!("SPIKE error tokio runtime: {error}");
            return;
        }
    };

    jni::init_platform(&app);
    let Some(platform) = jni::shared_platform() else {
        log::info!("SPIKE error gpui-pre-mobile platform missing");
        return;
    };

    // Blocks in gpui-pre-mobile's event loop until the activity is destroyed.
    // The closure runs once InitWindow has produced the one native window.
    Application::with_platform(platform.into_rc()).run(move |cx: &mut App| {
        let watcher = lifecycle::Watcher::new();
        let top_inset = lifecycle::top_inset();
        let (daemon, rx) = daemon::Daemon::new(handle, backend);
        let opened = cx.open_window(WindowOptions::default(), move |window, cx| {
            cx.new(|cx| ui::Library::new(daemon, rx, top_inset, window, cx))
        });
        match opened {
            Ok(window) => watcher.spawn(cx, window.into()),
            Err(error) => log::info!("SPIKE error open_window: {error:#}"),
        }
    });
    log::info!("SPIKE lifecycle exit");
}
