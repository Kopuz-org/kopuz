//! Debounced, revision-checked settings persistence for the app lifecycle.

use dioxus::prelude::*;
use tracing::Instrument;

const STORE_SAVE_SETTLE_MS: u64 = 600;
const STORE_SAVE_COOLDOWN_MS: u64 = 2500;

/// Pull the daemon's settings into the app's copy, keeping edits not yet sent.
pub(crate) async fn adopt_daemon_config(
    config: Signal<config::AppConfig>,
    baseline: hooks::config_sync::ConfigBaseline,
) {
    match crate::backend::api().config().await {
        Ok(view) => baseline.adopt(config, &view),
        Err(error) => tracing::warn!(%error, "re-reading settings failed"),
    }
}

pub(crate) fn use_settings_persistence(
    config: Signal<config::AppConfig>,
    config_baseline: hooks::config_sync::ConfigBaseline,
    initial_load_done: Signal<bool>,
    config_loaded_ok: Signal<bool>,
    persisted_volume: Signal<f32>,
    volume: Signal<f32>,
) {
    let mut config_dirty = use_signal(|| 0u64);
    use_effect(move || {
        if !*initial_load_done.read() || !*config_loaded_ok.read() {
            return;
        }
        let _ = config.read();
        config_dirty += 1;
    });
    use_effect(move || {
        if !*initial_load_done.read() || !*config_loaded_ok.read() {
            return;
        }
        let _ = *persisted_volume.read();
        config_dirty += 1;
    });
    #[cfg(not(target_os = "android"))]
    use_effect(move || {
        if !*initial_load_done.read() || !*config_loaded_ok.read() {
            return;
        }
        let mut snapshot = config.read().clone();
        let _ = *persisted_volume.read();
        snapshot.volume = *volume.peek();
        if let Some(update) = config_baseline.update(snapshot) {
            crate::exit_flush::stash_config(update);
        }
    });

    use_future(move || async move {
        let api = hooks::consume_api();
        let mut flushed = 0u64;
        loop {
            if *config_dirty.peek() == flushed {
                utils::sleep(std::time::Duration::from_millis(250)).await;
                continue;
            }
            utils::sleep(std::time::Duration::from_millis(STORE_SAVE_SETTLE_MS)).await;
            let pending = *config_dirty.peek();
            let mut snapshot = config.peek().clone();
            snapshot.volume = *volume.peek();
            let Some(update) = config_baseline.update(snapshot.clone()) else {
                continue;
            };
            match api
                .set_config(update)
                .instrument(tracing::info_span!("config.persist"))
                .await
            {
                Ok(view) => {
                    flushed = pending;
                    config_baseline.acknowledge(config, &snapshot, &view);
                }
                Err(error) => {
                    tracing::error!(%error, "failed to save settings");
                    if error.code == api::ErrorCode::Conflict {
                        adopt_daemon_config(config, config_baseline).await;
                    }
                }
            }
            utils::sleep(std::time::Duration::from_millis(STORE_SAVE_COOLDOWN_MS)).await;
        }
    });
}
