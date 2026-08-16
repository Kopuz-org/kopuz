use config::AppConfig;
use dioxus::prelude::*;
use tracing::Instrument;

pub fn use_connectivity_probe(
    config: Signal<AppConfig>,
    mut network_banner: Signal<Option<bool>>,
) -> Signal<bool> {
    let mut is_offline = use_signal(|| false);
    use_context_provider(|| is_offline);

    use_future(move || async move {
        let Ok(client) = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
        else {
            return;
        };
        let mut misses: u8 = 0;
        loop {
            if config.peek().server.is_none() {
                if *is_offline.peek() {
                    is_offline.set(false);
                }
                misses = 0;
                utils::sleep(std::time::Duration::from_secs(30)).await;
                continue;
            }
            let online = client
                .get("https://1.1.1.1")
                .send()
                .instrument(tracing::info_span!("net.connectivity"))
                .await
                .is_ok();
            if online {
                misses = 0;
                if *is_offline.peek() {
                    is_offline.set(false);
                }
            } else {
                misses = misses.saturating_add(1);
                if misses >= 2 && !*is_offline.peek() {
                    is_offline.set(true);
                }
            }
            let secs = if *is_offline.peek() { 10 } else { 30 };
            utils::sleep(std::time::Duration::from_secs(secs)).await;
        }
    });

    use_effect(move || {
        if *is_offline.read() {
            network_banner.set(Some(true));
        } else if network_banner.peek().as_ref() == Some(&true) {
            network_banner.set(Some(false));
            spawn(async move {
                utils::sleep(std::time::Duration::from_secs(4)).await;
                if network_banner.read().as_ref() == Some(&false) {
                    network_banner.set(None);
                }
            });
        }
    });

    is_offline
}
