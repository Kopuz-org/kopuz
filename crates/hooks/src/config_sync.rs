//! Keep the app's settings copy in step with the daemon's.

use dioxus::prelude::*;
use serde_json::{Map, Value};

/// Volume rides the player state, so the settings copy never takes it from here.
const SKIPPED: &[&str] = &["volume"];

/// The config this app last sent or adopted and the daemon revision it matches, which tells the echo of its own write from another writer's change.
#[derive(Clone, Copy)]
pub struct ConfigBaseline(Signal<Option<(config::AppConfig, u64)>>);

pub fn use_config_baseline_provider() -> ConfigBaseline {
    use_context_provider(|| ConfigBaseline(Signal::new(None)))
}

pub fn use_config_baseline() -> ConfigBaseline {
    use_context::<ConfigBaseline>()
}

impl ConfigBaseline {
    /// Start from the view the app loaded.
    pub fn loaded(mut self, view: &api::ConfigView) {
        self.0.set(Some((view.config.clone(), view.revision)));
    }

    /// Note the config this app just sent, before adopting the view its write answered with.
    pub fn sent(mut self, config: config::AppConfig) {
        let revision = self.0.peek().as_ref().map_or(0, |(_, revision)| *revision);
        self.0.set(Some((config, revision)));
    }

    /// Take into `local` every field a newer `view` moved since the baseline, keeping edits not yet sent.
    pub fn adopt(mut self, mut local: Signal<config::AppConfig>, view: &api::ConfigView) {
        let Some((baseline, revision)) = self.0.peek().clone() else {
            return;
        };
        // A read that left before a later write answered is older than what the app holds; a daemon that predates revisions sends 0 and is taken as is.
        if view.revision != 0 && view.revision <= revision {
            return;
        }
        let current = local.peek().clone();
        match moved(&current, &baseline, &view.config) {
            Some((next_local, next_baseline)) => {
                self.0.set(Some((next_baseline, view.revision)));
                local.set(next_local);
            }
            None => self.0.set(Some((baseline, view.revision))),
        }
    }
}

fn fields(config: &config::AppConfig) -> Option<Map<String, Value>> {
    match serde_json::to_value(config) {
        Ok(Value::Object(map)) => Some(map),
        Ok(_) => None,
        Err(error) => {
            tracing::warn!(%error, "settings did not serialize");
            None
        }
    }
}

fn rebuild(map: Map<String, Value>) -> Option<config::AppConfig> {
    serde_json::from_value(Value::Object(map))
        .map_err(|error| tracing::warn!(%error, "settings did not deserialize"))
        .ok()
}

/// `daemon`'s fields that differ from `baseline`, laid over `local` and over `baseline`; `None` when none moved.
fn moved(
    local: &config::AppConfig,
    baseline: &config::AppConfig,
    daemon: &config::AppConfig,
) -> Option<(config::AppConfig, config::AppConfig)> {
    let (mut local, mut baseline) = (fields(local)?, fields(baseline)?);
    let mut changed = false;
    for (key, value) in fields(daemon)? {
        if SKIPPED.contains(&key.as_str()) || baseline.get(&key) == Some(&value) {
            continue;
        }
        local.insert(key.clone(), value.clone());
        baseline.insert(key, value);
        changed = true;
    }
    if !changed {
        return None;
    }
    Some((rebuild(local)?, rebuild(baseline)?))
}

#[cfg(test)]
mod tests {
    use super::moved;

    fn config(crossfade: u8, lyrics: bool) -> config::AppConfig {
        config::AppConfig {
            crossfade_seconds: crossfade,
            prefer_local_lyrics: lyrics,
            ..Default::default()
        }
    }

    #[test]
    fn the_echo_of_a_write_does_not_undo_an_edit_made_since() {
        let sent = config(4, false);
        let local = config(5, false);
        assert!(moved(&local, &sent, &sent).is_none());
    }

    #[test]
    fn another_writers_change_lands_beside_an_unsent_edit() {
        let baseline = config(4, false);
        let local = config(5, false);
        let daemon = config(4, true);
        let (next, base) = moved(&local, &baseline, &daemon).expect("a field moved");
        assert_eq!(
            (next.crossfade_seconds, next.prefer_local_lyrics),
            (5, true)
        );
        assert_eq!(
            (base.crossfade_seconds, base.prefer_local_lyrics),
            (4, true)
        );
    }

    #[test]
    fn a_field_only_the_daemon_writes_reaches_the_copy() {
        let baseline = config(4, false);
        let mut daemon = baseline.clone();
        daemon.offline_tracks.insert("k".into(), "/cache/k".into());
        let (next, _) = moved(&baseline, &baseline, &daemon).expect("downloads moved");
        assert_eq!(
            next.offline_tracks.get("k").map(String::as_str),
            Some("/cache/k")
        );
    }

    #[test]
    fn volume_never_comes_from_the_settings_copy() {
        let baseline = config(4, false);
        let daemon = config::AppConfig {
            volume: 0.1,
            ..baseline.clone()
        };
        assert!(moved(&baseline, &baseline, &daemon).is_none());
    }
}
