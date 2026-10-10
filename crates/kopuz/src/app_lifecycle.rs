use dioxus::prelude::*;
use tracing::Instrument;

/// The JS runtime the daemon borrows from this process. Desktop runs its own;
/// on Android the only capable runtime is the WebView already drawing the UI,
/// so the daemon's solve requests are drained through `document::eval`. Which
/// source needed a script, and why, stays behind the seam.
#[cfg(target_os = "android")]
pub fn use_webview_script_engine() {
    use_hook(|| {
        let (engine, mut rx) = daemon::script_engine::webview_channel();
        if daemon::script_engine::set_engine(engine).is_err() {
            tracing::warn!("a script engine is already registered; the webview one is not active");
        }
        spawn(async move {
            while let Some(req) = rx.recv().await {
                let wrapped = format!(
                    "globalThis.print=function(s){{dioxus.send(s);}};\
                     try{{{}}}catch(e){{dioxus.send('\\u0000ERR'+(e&&e.stack?e.stack:e));}}",
                    req.program
                );
                let mut eval = dioxus::document::eval(&wrapped);
                let result = match tokio::time::timeout(
                    std::time::Duration::from_secs(20),
                    eval.recv::<String>(),
                )
                .await
                {
                    Ok(Ok(s)) => match s.strip_prefix('\u{0}') {
                        Some(err) => Err(format!("webview JS: {}", err.trim_start_matches("ERR"))),
                        None => Ok(s),
                    },
                    Ok(Err(error)) => Err(format!("webview eval recv: {error}")),
                    Err(_) => Err("webview decipher timed out".to_string()),
                };
                let _ = req.reply.send(result);
            }
        });
    });
}

/// Probe rounds that must fail in a row before the app calls itself offline.
const OFFLINE_AFTER_MISSES: u8 = 3;

/// Consecutive-miss bookkeeping for the connectivity probe, kept apart from the
/// signal so the decision can be tested.
#[derive(Debug, Default)]
struct Reachability {
    misses: u8,
    offline: bool,
}

impl Reachability {
    fn record(&mut self, reached: bool) -> bool {
        if reached {
            self.misses = 0;
            self.offline = false;
        } else {
            self.misses = self.misses.saturating_add(1);
            if self.misses >= OFFLINE_AFTER_MISSES {
                self.offline = true;
            }
        }
        self.offline
    }

    /// Any miss, online or offline, is rechecked after 10s: a real outage is
    /// reported in about 30s and recovery is noticed as quickly.
    fn next_probe(&self) -> std::time::Duration {
        std::time::Duration::from_secs(if self.misses > 0 { 10 } else { 30 })
    }
}

/// Bare IPs so the probe never depends on DNS. Redirects are not followed:
/// 1.1.1.1 redirects to one.one.one.one, which filtering resolvers such as
/// Cisco Umbrella sinkhole as a DoH endpoint, failing TLS on a healthy network. Any HTTP answer from either host proves a route out.
const PROBE_TARGETS: &[&str] = &["https://1.1.1.1", "https://8.8.8.8"];

fn probe_targets() -> Vec<String> {
    #[cfg(debug_assertions)]
    if let Ok(list) = std::env::var("KOPUZ_CONNECTIVITY_PROBES") {
        return list.split(',').map(|s| s.trim().to_string()).collect();
    }
    PROBE_TARGETS.iter().map(|s| s.to_string()).collect()
}

async fn probe(client: &reqwest::Client, targets: &[String]) -> bool {
    let attempts = targets.iter().map(|url| {
        Box::pin(async move {
            client.get(url).send().await.map_err(|error| {
                tracing::debug!(url, %error, "connectivity probe target failed");
            })
        })
    });
    futures_util::future::select_ok(attempts).await.is_ok()
}

pub fn use_connectivity_probe(mut network_banner: Signal<Option<bool>>) -> Signal<bool> {
    let mut is_offline = use_signal(|| false);
    use_context_provider(|| is_offline);
    // Only a remote source makes reachability a thing worth watching, and
    // which is active is the daemon's answer.
    let active = hooks::sources::use_active_source_info();
    use_future(move || async move {
        let Ok(client) = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .build()
        else {
            return;
        };
        let targets = probe_targets();
        let mut state = Reachability::default();
        loop {
            if !active
                .peek()
                .as_ref()
                .is_some_and(|source| source.needs_network)
            {
                if *is_offline.peek() {
                    is_offline.set(false);
                }
                state = Reachability::default();
                utils::sleep(std::time::Duration::from_secs(30)).await;
                continue;
            }
            let reached = probe(&client, &targets)
                .instrument(tracing::info_span!("net.connectivity"))
                .await;
            let offline = state.record(reached);
            tracing::debug!(
                reached,
                misses = state.misses,
                offline,
                "connectivity probe"
            );
            if offline != *is_offline.peek() {
                tracing::info!(offline, "connectivity changed");
                is_offline.set(offline);
            }
            utils::sleep(state.next_probe()).await;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_blip_stays_online() {
        let mut state = Reachability::default();
        assert!(!state.record(true));
        assert!(!state.record(false));
        assert!(!state.record(true));
        assert!(!state.record(false));
        assert!(!state.record(false));
        assert!(!state.record(true));
    }

    #[test]
    fn sustained_failure_goes_offline() {
        let mut state = Reachability::default();
        assert!(!state.record(false));
        assert!(!state.record(false));
        assert!(state.record(false));
        assert!(state.record(false));
    }

    #[test]
    fn recovery_goes_back_online() {
        let mut state = Reachability::default();
        for _ in 0..OFFLINE_AFTER_MISSES {
            state.record(false);
        }
        assert!(state.offline);
        assert!(!state.record(true));
        assert_eq!(state.misses, 0);
        assert!(!state.record(false));
    }

    #[test]
    fn misses_are_rechecked_sooner() {
        let mut state = Reachability::default();
        assert_eq!(state.next_probe().as_secs(), 30);
        state.record(false);
        assert_eq!(state.next_probe().as_secs(), 10);
        state.record(true);
        assert_eq!(state.next_probe().as_secs(), 30);
    }
}
