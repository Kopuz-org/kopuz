//! Calls the integration plugins that listen to `tick` every `tick-ms` their
//! manifest asks for. A tick still running when the next is due makes that one
//! skip, so a slow plugin is called less often rather than piling up.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use plugin_host::Integration;
use plugin_host::manifest::Listen;
use tokio::time::Instant;

use crate::plugin_integrations::PluginIntegrations;

/// The finest a tick can land; the shortest `tick-ms` is a hundred.
const RESOLUTION: Duration = Duration::from_millis(50);

/// How often the ticking plugins are looked up again.
const RECHECK: Duration = Duration::from_secs(1);

struct Ticking {
    integration: Arc<Integration>,
    every: Duration,
    due: Instant,
    busy: Arc<AtomicBool>,
}

/// Marks a plugin's tick finished when dropped.
struct Idle(Arc<AtomicBool>);

impl Drop for Idle {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn interval(integration: &Integration) -> Option<Duration> {
    let plugin = integration.plugin();
    let spec = plugin.manifest().integration.as_ref()?;
    spec.listens_to(Listen::Tick)
        .then(|| Duration::from_millis(spec.tick_ms.into()))
}

pub fn spawn(plugins: Arc<PluginIntegrations>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticking: HashMap<String, Ticking> = HashMap::new();
        let mut rechecked: Option<Instant> = None;
        loop {
            let now = Instant::now();
            if rechecked.is_none_or(|at| now.duration_since(at) >= RECHECK) {
                rechecked = Some(now);
                let running = plugins.running();
                ticking.retain(|_, held| {
                    running
                        .iter()
                        .any(|integration| Arc::ptr_eq(integration, &held.integration))
                });
                for integration in running {
                    let Some(every) = interval(&integration) else {
                        continue;
                    };
                    let id = integration.plugin().id().to_string();
                    ticking.entry(id).or_insert_with(|| Ticking {
                        integration,
                        every,
                        due: now,
                        busy: Arc::new(AtomicBool::new(false)),
                    });
                }
            }
            if ticking.is_empty() {
                tokio::time::sleep(RECHECK).await;
                continue;
            }
            for held in ticking.values_mut() {
                // Replaced or turned off; the next look-up drops it.
                if now < held.due || held.integration.is_closed() {
                    continue;
                }
                held.due = now + held.every;
                if held.busy.swap(true, Ordering::AcqRel) {
                    continue;
                }
                let (integration, busy) = (Arc::clone(&held.integration), Arc::clone(&held.busy));
                tokio::spawn(async move {
                    // Cleared however the tick ends, a panic or a dropped task included,
                    // so one bad tick never stops the next.
                    let _idle = Idle(busy);
                    if let Err(error) = integration.on_tick().await {
                        tracing::warn!(plugin = %integration.plugin().id(), %error, "an integration's tick failed");
                    }
                });
            }
            tokio::time::sleep(RESOLUTION).await;
        }
    })
}
